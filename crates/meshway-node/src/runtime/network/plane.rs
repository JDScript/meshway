//! The Node's component set behind the single [`meshway_engine::Engine`].
//! Each apply diffs the actual installed resources against the desired
//! content. Withdrawal terminates every flow, circuit and exit projection,
//! closes adjacencies and keeps only the negative capture and DNS boundaries.
use super::{Report, dns::Attachment, packets::Packets};
use crate::host::Host;
use meshway_configuration::netmap::DnsPlan;
use meshway_connectivity::Manager;
use meshway_dataplane::secure::SecureNode;
use meshway_engine::DataPlaneKeys;
use meshway_engine::{Desired, Error};
use meshway_protocol::ipc::Problem;
use meshway_protocol::{InstanceRequest, execution::ApplyFailure};
use std::sync::Arc;

pub(super) struct Installed {
    pub desired: Desired,
    /// Absent for a disabled Node, which owns no packet plane.
    pub secure: Option<SecureNode>,
    pub manager: Option<Manager>,
    pub exit: Option<crate::host::ExitAttachment>,
}

pub(super) struct Plane {
    pub host: Box<dyn Host>,
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
    /// The boot of a Manager restarted after a failed apply while SecureNode
    /// kept its circuits: the next Manager signs the same one, so no peer
    /// sees a restart of this Node (see `SecureNode::set_own_boot`).
    boot: Option<meshway_connectivity::Boot>,
}

impl Plane {
    pub fn new(
        host: Box<dyn Host>,
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
            boot: None,
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
            .probe_exit(instance, reference, role.generation, &role.families)
            .await;
        capability.set_installed(installed);
        self.report.exit_capability = Some(capability.clone());
        Some(capability)
    }

    /// End only the installed exit role: its flows retire with their
    /// projections; every other component keeps the applied content.
    pub async fn withdraw_exit(&mut self) -> Result<(), Problem> {
        let Some(state) = self.installed.as_mut() else {
            return Ok(());
        };
        let Some(exit) = state.exit.take() else {
            return Ok(());
        };
        let result = match &mut state.secure {
            Some(secure) => exit.stop(secure).await,
            None => Err(Problem::Underlay),
        };
        if let Some(capability) = &mut self.report.exit_capability {
            capability.set_installed(false);
        }
        result
    }

    /// The applied content assigns this Node an exit role it has not
    /// installed (its prerequisites were unavailable when it was applied).
    pub fn exit_pending(&self) -> bool {
        self.installed.as_ref().is_some_and(|state| {
            state.secure.is_some() && state.exit.is_none() && state.desired.exit_role().is_some()
        })
    }

    /// Whether a configured listener of the installed content is unbound
    /// (its Manager owner retries the bind).
    pub fn listeners_unavailable(&self) -> bool {
        self.installed
            .as_ref()
            .and_then(|state| state.manager.as_ref())
            .is_some_and(|manager| !manager.unavailable_listeners().is_empty())
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
        self.boot = None;
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
                match self.host.start_exit(plan.clone()) {
                    Ok(role) => role,
                    Err(problem) => return Err(self.fail(problem, ApplyFailure::Underlay)),
                }
            }
            _ => None,
        };
        let state = self.installed.as_mut().expect("checked above");
        state.secure = Some(secure);
        if let Some(role) = role {
            state.exit = Some(role);
        }
        let state = self.installed.as_mut().expect("checked above");
        if let (Some(exit), Some(secure)) = (&mut state.exit, &mut state.secure) {
            if let Err(problem) = exit.synchronize(secure).await {
                return Err(self.fail(problem, ApplyFailure::Underlay));
            }
            if let Some(capability) = &mut self.report.exit_capability {
                capability.set_installed(true);
            }
        }
        Ok(())
    }

    /// Authenticated adjacencies: unchanged resources and channels survive.
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            self.boot = None;
            if let Some(state) = &mut self.installed
                && let Some(mut manager) = state.manager.take()
            {
                let _ = manager.shutdown().await;
            }
            return Ok(());
        };
        if !desired.local().enabled {
            self.boot = None;
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
                    self.boot = Some(manager.boot());
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
                warn_ephemeral(desired, self.host.as_ref());
                // Carried only while the circuits it was signed for live on.
                let boot = self.boot.take().filter(|_| {
                    self.installed
                        .as_ref()
                        .is_some_and(|state| state.secure.is_some())
                });
                match Manager::start_with_traversal(
                    desired,
                    keys,
                    self.host.sockets(),
                    Default::default(),
                    self.traversal.clone(),
                    generation,
                    self.instance.clone(),
                    boot,
                )
                .await
                {
                    Ok(manager) => manager,
                    Err(error) => {
                        tracing::warn!(%error, "Adjacency start failed");
                        self.boot = boot;
                        self.failure = Some((Problem::Transport, ApplyFailure::Listeners));
                        self.failure_detail = detail(&error);
                        return Err(Error::Degraded);
                    }
                }
            }
        };
        // A listener whose bind its owner retries leaves the content
        // installed without it until it binds.
        let unbound = manager.unavailable_listeners();
        manager.attach_facts(self.peers.clone());
        if let Some(state) = &mut self.installed {
            state.manager = Some(manager);
        }
        if let Some(listener) = unbound.first() {
            self.failure = Some((Problem::Transport, ApplyFailure::Listeners));
            self.failure_detail = Some(unavailable_detail(listener, desired, self.host.as_ref()));
            tracing::warn!(
                key = listener.key,
                detail = self.failure_detail.as_deref(),
                "Content installed without a listener"
            );
            return Err(Error::Degraded);
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
            self.failure_detail = self.host.ownership_ended().map(|end| end.to_string());
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
            self.failure_detail = self.dns.detail();
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
                    .probe_exit(
                        instance.instance_id,
                        desired.reference(),
                        role.generation,
                        &role.families,
                    )
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
        // This Node's exit role is a capability it reports, not a component
        // its own traffic requires: unmet prerequisites or a failed role
        // installation leave `exitCapability` unavailable and the rest of the
        // content applied. The periodic capability probe installs the role
        // once its prerequisites hold (see `exit_pending`).
        if let Some(role) = &netmap.exit_role
            && role_changed
        {
            let status = self
                .report
                .exit_capability
                .as_ref()
                .map(|capability| &capability.families);
            if !self
                .report
                .exit_capability
                .as_ref()
                .is_some_and(meshway_model::exit::ExitCapability::installable)
            {
                // No status yet: content applied from the saved netmap before
                // the instance its observation binds to was known.
                tracing::info!(
                    ?status,
                    "Exit role not installed yet: prerequisites not observed ready"
                );
            } else if let Err(error) = self.host.install_netmap_exit_role(role).await {
                tracing::warn!(%error, "Exit role installation failed");
                if let Some(capability) = &mut self.report.exit_capability {
                    capability.set_status(meshway_model::exit::ExitStatus::Unavailable {
                        reason: meshway_model::exit::ExitUnavailableReason::OwnershipConflict,
                    });
                }
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

/// An unbound listener's cause, naming this host's ephemeral port range
/// when the port is inside it (another socket may hold it for a moment)
/// and which processes hold it (this daemon among them is named so).
fn unavailable_detail(
    listener: &meshway_connectivity::ListenerUnavailable,
    desired: &meshway_engine::Desired,
    host: &dyn Host,
) -> String {
    let mut detail = listener.detail.clone();
    let Some(address) = listener.address else {
        return detail + "; retrying";
    };
    let udp = desired
        .netmap()
        .self_
        .listeners
        .get(&listener.key)
        .is_some_and(|declaration| {
            matches!(
                declaration.declaration(),
                meshway_configuration::transport::Listener::NativeUdp(_)
            )
        });
    let holders = host.port_holders(address.port(), udp);
    if !holders.is_empty() {
        let own = std::process::id();
        let names: Vec<_> = holders
            .iter()
            .take(4)
            .map(|(pid, command)| match *pid == own {
                true => format!("{command} pid {pid} (this daemon)"),
                false => format!("{command} pid {pid}"),
            })
            .collect();
        detail += &format!("; held by {}", names.join(", "));
    }
    if let Some(range) = host
        .ephemeral_ports()
        .filter(|range| range.contains(&address.port()))
    {
        detail += &format!(
            "; port {} is inside this host's ephemeral port range {}-{}",
            address.port(),
            range.start(),
            range.end()
        );
    }
    detail + "; retrying"
}

/// Listener ports another process's outgoing socket can hold for a moment.
fn warn_ephemeral(desired: &meshway_engine::Desired, host: &dyn Host) {
    let Some(range) = host.ephemeral_ports() else {
        return;
    };
    for (key, listener) in &desired.netmap().self_.listeners {
        let (bind, enabled) = listener.declaration().bind();
        if enabled && range.contains(&bind.port()) {
            tracing::warn!(
                key,
                port = bind.port(),
                first = range.start(),
                last = range.end(),
                "Listener port is inside this host's ephemeral port range; its bind may need a retry"
            );
        }
    }
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
