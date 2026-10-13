//! The Node's component set behind the single [`meshway_engine::Engine`].
//! Each apply diffs the actual installed resources against the desired
//! content. Withdrawal terminates every flow, circuit and exit projection,
//! closes adjacencies and keeps only the negative capture and DNS boundaries.
use super::{Report, dns::Attachment, packets::Packets};
use meshway_configuration::netmap::DnsPlan;
use meshway_connectivity::Manager;
use meshway_dataplane::secure::SecureNode;
use meshway_engine::DataPlaneKeys;
use meshway_engine::{Desired, Error};
use meshway_protocol::ipc::Problem;
use meshway_protocol::{InstanceRequest, execution::ApplyFailure};
use meshway_underlay::Host;
use std::sync::Arc;

pub(super) struct Installed {
    pub desired: Desired,
    /// Absent for a disabled Node, which owns no packet plane.
    pub secure: Option<SecureNode>,
    pub manager: Option<Manager>,
    pub exit: Option<super::exit::Runtime>,
}

pub(super) struct Plane {
    pub host: Host,
    pub dns: Attachment,
    /// Control's published last known peer facts (selection input only).
    pub peers: tokio::sync::watch::Receiver<Option<meshway_control::KnownPeers>>,
    pub packets: Packets,
    pub installed: Option<Installed>,
    /// Working report fields owned by the applies; the worker publishes them.
    pub report: Report,
    /// Why the last failed apply failed.
    pub failure: Option<(Problem, ApplyFailure)>,
    /// The concrete cause of `failure`, when the failing component names one.
    pub failure_detail: Option<String>,
    traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    keys: Option<DataPlaneKeys>,
    instance: Option<InstanceRequest>,
    generation: meshway_model::Sequence,
}

impl Plane {
    pub fn new(
        host: Host,
        dns: Attachment,
        peers: tokio::sync::watch::Receiver<Option<meshway_control::KnownPeers>>,
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
            failure_detail: None,
            traversal,
            keys: None,
            instance: None,
            generation: meshway_model::Sequence::MIN,
        })
    }

    pub fn set_keys(&mut self, keys: Option<DataPlaneKeys>) {
        self.keys = keys;
    }

    /// State/receipt binding only; never part of desired content. Returns
    /// whether the binding changed.
    pub async fn set_instance(&mut self, instance: Option<InstanceRequest>) -> bool {
        if self.instance == instance {
            return false;
        }
        self.instance = instance;
        if let Some(manager) = self
            .installed
            .as_mut()
            .and_then(|state| state.manager.as_mut())
            && let Err(error) = manager.set_instance(self.instance.clone()).await
        {
            // Coordination sockets stay plain until the next apply rebinds.
            tracing::warn!(?error, "Adjacency instance binding failed");
        }
        true
    }

    /// The installed exit role's prerequisites, observed for the live
    /// instance (an installation may precede the instance, e.g. after a
    /// restart on the saved netmap). Facts only; nothing is reinstalled.
    pub async fn exit_capability(&mut self) -> Option<meshway_model::exit::ExitCapability> {
        let state = self.installed.as_ref()?;
        let role = state.desired.exit_role()?.clone();
        let reference = state.desired.reference();
        let installed = state.exit.is_some();
        let instance = self.instance.as_ref()?.instance_id;
        let mut capability = self
            .host
            .probe_exit(instance, reference, role.generation)
            .await;
        if installed {
            capability.installed_families = vec![meshway_model::exit::ExitFamily::Ipv4];
        }
        self.report.exit_capability = Some(capability.clone());
        Some(capability)
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

    #[track_caller]
    fn fail(&mut self, problem: Problem, failure: ApplyFailure) -> Error {
        tracing::debug!(
            ?problem,
            ?failure,
            at = %std::panic::Location::caller(),
            "Component apply failed"
        );
        self.failure = Some((problem, failure));
        Error::Apply
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
    fn exit_destinations(&self) -> std::collections::BTreeSet<std::net::IpAddr> {
        self.secure()
            .map(meshway_dataplane::secure::SecureNode::exit_destinations)
            .unwrap_or_default()
    }
    /// Flows, circuits, queued outputs and the exit forwarding owner.
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            self.dns.suspend();
            let closing = self.close().await;
            return closing.map_err(|problem| self.fail(problem, ApplyFailure::Underlay));
        };
        self.packets.clear_admitted();
        if !desired.local().enabled {
            return Ok(());
        }
        let Some(keys) = self.keys.clone() else {
            return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
        };
        let Some(state) = self.installed.as_mut() else {
            return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
        };
        // No running Manager (a degraded adjacency apply) is no carrier.
        let carriers = state
            .manager
            .as_ref()
            .and_then(|manager| manager.carriers().ok())
            .unwrap_or_default();
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
                // No installed role (a degraded host apply): no exit runtime,
                // so this Node forwards nothing to external destinations.
                self.host.exit_role().map(|role| (role, plan.clone()))
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
                let _ = manager.shutdown().await;
            }
            return Ok(());
        };
        if !desired.local().enabled {
            if let Some(state) = &mut self.installed
                && let Some(mut manager) = state.manager.take()
            {
                let _ = manager.shutdown().await;
            }
            return Ok(());
        }
        let previous = self
            .installed
            .as_mut()
            .and_then(|state| state.manager.take());
        // Adjacency failures only withhold capability: SecureNode admits
        // nothing more for a missing or partially bound Manager. A Manager is
        // kept only while it runs on exactly this content.
        let manager = match previous {
            Some(mut manager) => match manager.apply(desired).await {
                Ok(()) => manager,
                Err(error)
                    if manager.is_running() && manager.reference() == desired.reference() =>
                {
                    tracing::warn!(%error, "Adjacency resources partially unavailable");
                    self.failure = Some((Problem::Transport, ApplyFailure::Listeners));
                    self.failure_detail = detail(&error);
                    manager.attach_facts(self.peers.clone());
                    if let Some(state) = &mut self.installed {
                        state.manager = Some(manager);
                    }
                    return Err(Error::Degraded);
                }
                Err(error) => {
                    tracing::warn!(%error, "Adjacency apply failed; restarting adjacencies");
                    let _ = manager.shutdown().await;
                    self.failure = Some((Problem::Transport, ApplyFailure::Listeners));
                    self.failure_detail = detail(&error);
                    return Err(Error::Degraded);
                }
            },
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
                    self.instance.clone(),
                )
                .await
                {
                    Ok(manager) => manager,
                    Err(error) => {
                        tracing::warn!(%error, "Adjacency start failed");
                        self.failure = Some((Problem::Transport, ApplyFailure::Listeners));
                        self.failure_detail = detail(&error);
                        return Err(Error::Degraded);
                    }
                }
            }
        };
        manager.attach_facts(self.peers.clone());
        if let Some(state) = &mut self.installed {
            state.manager = Some(manager);
        }
        Ok(())
    }

    /// Capture, TUN, kept descriptors, negative DNS and the exit role.
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            return match self.host.suspend().await {
                Ok(()) => Ok(()),
                Err(_) => Err(self.fail(Problem::Underlay, ApplyFailure::Underlay)),
            };
        };
        if let Err(error) = self.host.install(desired.clone()).await {
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
        // An exit role that cannot be installed is a missing capability, not
        // a safety failure: the content stays installed without it.
        let mut degraded = false;
        if let Some(role) = &netmap.exit_role
            && role_changed
        {
            if self
                .report
                .exit_capability
                .as_ref()
                .is_some_and(|c| c.status != meshway_model::exit::ExitStatus::Ready)
            {
                degraded = true;
            } else if let Err(error) = self.host.install_netmap_exit_role(role).await {
                tracing::warn!(%error, "Exit role installation failed");
                if let Some(capability) = &mut self.report.exit_capability {
                    capability.status = meshway_model::exit::ExitStatus::Unavailable {
                        reason: meshway_model::exit::ExitUnavailableReason::OwnershipConflict,
                    };
                }
                degraded = true;
            }
        }
        if self.host.tun().is_none() {
            return Err(self.fail(Problem::Tun, ApplyFailure::Tun));
        }
        match &mut self.installed {
            Some(state) => state.desired = desired.clone(),
            None => {
                self.installed = Some(Installed {
                    desired: desired.clone(),
                    secure: None,
                    manager: None,
                    exit: None,
                })
            }
        }
        if degraded {
            self.failure = Some((Problem::Underlay, ApplyFailure::Underlay));
            return Err(Error::Degraded);
        }
        Ok(())
    }

    /// Query authority over the packet plane, after the negative capture.
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            self.dns.suspend();
            return Ok(());
        };
        let netmap = desired.netmap();
        let needs_dns = super::requirements::dns_required(&netmap.projection.configuration.dns)
            || !matches!(netmap.dns.egress, DnsPlan::NoExit);
        if needs_dns
            && netmap.self_.enabled
            && let Err(error) = self.dns.install(desired.clone()).await
        {
            // Answers are capability; the negative boundary is in apply_host.
            tracing::warn!(%error, "DNS scope installation failed");
            self.failure = Some((Problem::Dns, ApplyFailure::Dns));
            return Err(Error::Degraded);
        }
        Ok(())
    }
}

/// The part of an adjacency failure an operator acts on: which listener
/// cannot bind which address, and the system's reason.
fn detail(error: &meshway_connectivity::Error) -> Option<String> {
    matches!(error, meshway_connectivity::Error::Bind { .. }).then(|| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_bind_failure_names_its_cause() {
        let busy = meshway_connectivity::Error::Bind {
            key: "lab-tcp".into(),
            address: "0.0.0.0:443".parse().unwrap(),
            source: std::io::ErrorKind::AddrInUse.into(),
        };
        let detail = super::detail(&busy).unwrap();
        assert!(detail.starts_with("listener lab-tcp cannot bind 0.0.0.0:443: "));
        assert!(super::detail(&meshway_connectivity::Error::Underlay).is_none());
    }
}
