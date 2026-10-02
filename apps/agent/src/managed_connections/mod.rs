//! Explicitly enabled adjacency application. Packet grants require a separately
//! validated coherent bundle and the managed-dataplane opt-in.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use futures_util::{StreamExt, stream};
use meshway_dataplane::{
    LabConfig, ListenerConfig, OutgoingCandidate, PeerConfig, PrivateKey, TransportKind,
};
use meshway_protocol::{
    AgentConnectionsSnapshot, ConnectionRuntimeReason as Reason, ConnectionRuntimeState as Phase,
    ConnectionTransport, NodeConnections,
};
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{self, Instant},
};

use crate::{
    AgentError,
    managed_dataplane::{PacketPlan, TunOptions, tun::Tun},
};

mod discovery;
#[cfg(test)]
mod managed_material_tests;
#[cfg(unix)]
mod probe;
mod profiles;
mod session;
mod sockets;
#[cfg(unix)]
pub(crate) use probe::Prober;
#[cfg(test)]
mod packet_tests;
#[cfg(all(test, unix))]
mod probe_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
impl Manager {
    pub(crate) fn mock_tun(&mut self, tun: Tun) {
        self.tun = Some(tun);
    }
}

#[derive(Clone, Copy)]
struct Failure(Reason);

#[derive(Clone, serde::Serialize)]
pub(crate) struct Observation {
    pub state: Phase,
    pub reason: Reason,
    pub listeners: Vec<meshway_protocol::ConnectionListenerObservation>,
    pub peers: Vec<meshway_protocol::ConnectionNeighborObservation>,
}

impl Observation {
    fn empty(state: Phase, reason: Reason) -> Self {
        Self {
            state,
            reason,
            listeners: Vec::new(),
            peers: Vec::new(),
        }
    }
}

struct Running {
    /// Revision actually installed in this generation, independent of saved ACKs.
    revision: u32,
    deadline: watch::Sender<Option<Instant>>,
    observed: watch::Receiver<Observation>,
    task: JoinHandle<()>,
    discovery: watch::Receiver<discovery::Publication>,
    discovery_task: Option<JoinHandle<()>>,
}

pub(crate) struct ReportGuard {
    deadline: watch::Receiver<Option<Instant>>,
    observed: watch::Receiver<Observation>,
    observation_expiry: Option<Instant>,
}

impl ReportGuard {
    pub fn current(&self) -> bool {
        self.deadline
            .borrow()
            .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
            && !self.observed.has_changed().unwrap_or(true)
            && !self.deadline.has_changed().unwrap_or(true)
            && self
                .observation_expiry
                .is_none_or(|expiry| expiry > Instant::now())
    }

    pub async fn changed(&mut self) {
        if !self.current() {
            return;
        }
        let deadline = (*self.deadline.borrow()).expect("checked active deadline");
        let deadline = self
            .observation_expiry
            .map_or(deadline, |expiry| expiry.min(deadline));
        tokio::select! {
            _ = time::sleep_until(deadline) => {},
            _ = self.deadline.changed() => {},
            _ = self.observed.changed() => {},
        }
    }
}

impl Running {
    fn active(&self) -> bool {
        !self.task.is_finished()
            && self
                .deadline
                .borrow()
                .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
    }

    async fn stop(self) {
        let _ = self.deadline.send(None);
        if let Some(task) = self.discovery_task {
            task.abort();
            let _ = task.await;
        }
        let _ = self.task.await;
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        // The independently owned watchdog completes socket/task shutdown even
        // when the controlling HTTP future is cancelled or returns an error.
        if let Some(running) = &self.running {
            let _ = running.deadline.send(None);
        }
    }
}

pub(crate) struct Manager {
    socket_protection: std::sync::Arc<dyn sockets::Protection>,
    profiles: Option<PathBuf>,
    managed_materials: Vec<meshway_protocol::ManagedVlessMaterial>,
    lease: Duration,
    running: Option<Running>,
    semantic: Option<NodeConnections>,
    material: Option<Material>,
    observation: Observation,
    pub snapshot: Option<AgentConnectionsSnapshot>,
    dataplane: Option<DataplaneSettings>,
    packet_digest: Option<String>,
    tun: Option<Tun>,
    discovery_options: crate::EndpointDiscoveryOptions,
    discovery_generation: u32,
    discovery_timestamps: std::sync::Arc<discovery::TimestampGate>,
    #[cfg(unix)]
    probes: probe::Publisher,
}

struct DataplaneSettings {
    options: TunOptions,
    controller: String,
    may_reuse_guard: bool,
}

#[derive(PartialEq, Eq)]
struct Material {
    profiles: [u8; 32],
    targets: BTreeMap<String, Vec<SocketAddr>>,
}

impl Manager {
    pub fn new(profiles: Option<PathBuf>, lease: Duration) -> Self {
        Self {
            socket_protection: std::sync::Arc::new(sockets::SystemUnderlay),
            profiles,
            managed_materials: Vec::new(),
            lease,
            running: None,
            semantic: None,
            material: None,
            observation: Observation::empty(Phase::Withdrawn, Reason::None),
            snapshot: None,
            dataplane: None,
            packet_digest: None,
            tun: None,
            discovery_options: crate::EndpointDiscoveryOptions::default(),
            discovery_generation: 0,
            discovery_timestamps: std::sync::Arc::default(),
            #[cfg(unix)]
            probes: probe::Publisher::new(),
        }
    }

    pub fn new_dataplane(
        profiles: Option<PathBuf>,
        lease: Duration,
        options: TunOptions,
        controller: String,
        may_reuse_guard: bool,
    ) -> Self {
        let mut manager = Self::new(profiles, lease);
        manager.dataplane = Some(DataplaneSettings {
            options,
            controller,
            may_reuse_guard,
        });
        manager
    }

    pub fn enable_discovery(&mut self, options: crate::EndpointDiscoveryOptions) {
        self.discovery_options = options;
    }

    pub fn discovery_snapshot(
        &self,
    ) -> (
        meshway_protocol::EndpointDiscoveryState,
        Vec<meshway_protocol::UdpSocketDiscovery>,
        Option<ReportGuard>,
    ) {
        use meshway_protocol::EndpointDiscoveryState as State;
        if self.discovery_options.servers.is_empty() {
            return (State::Disabled, Vec::new(), self.report_snapshot().1);
        }
        let Some(running) = self.running.as_ref().filter(|r| r.active()) else {
            return (State::Withdrawn, Vec::new(), None);
        };
        let mut publication = running.discovery.borrow().clone();
        if !self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.payload.revision == publication.source.revision
                && snapshot.digest == publication.source.digest
        }) {
            return (State::Withdrawn, Vec::new(), None);
        }
        publication.prune();
        let (_, mut guard) = self.report_snapshot();
        if let Some(guard) = &mut guard {
            guard.observation_expiry = publication.earliest_expiry();
        }
        (publication.state, publication.sockets, guard)
    }

    /// Complete generation teardown immediately after validation, before disk
    /// persistence or any further HTTP request can stall. Runtime shutdown also
    /// destroys all established endpoint sessions and queued plaintext delivery.
    pub async fn invalidate_dataplane(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) {
        if self.packet_digest.as_ref() != Some(&snapshot.digest)
            || !snapshot.payload.access_valid
            || !snapshot.payload.configuration.policy_valid
            || !snapshot.payload.connections.valid
        {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
    }

    /// A validated publication can prove this generation stale before the next
    /// coherent GET completes. It never renews a lease or relies on disk receipts.
    pub async fn invalidate_dataplane_revision(&mut self, revision: u32) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| revision > running.revision)
        {
            self.withdraw(Reason::InvalidConfiguration).await;
        }
    }

    pub async fn reconcile_dataplane(
        &mut self,
        snapshot: meshway_protocol::AgentDataplaneSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) -> Result<(), AgentError> {
        self.invalidate_dataplane(&snapshot).await;
        self.prepare_dataplane_sink(&snapshot).await?;
        self.packet_digest = Some(snapshot.digest);
        let payload = snapshot.payload;
        let connections = AgentConnectionsSnapshot {
            digest: meshway_protocol::connection_digest(&payload.connections)
                .map_err(|_| AgentError::Invalid("cannot hash coherent adjacency projection"))?,
            payload: payload.connections.clone(),
        };
        if !payload.access_valid
            || !payload.configuration.policy_valid
            || !payload.connections.valid
        {
            self.snapshot = Some(connections);
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return Ok(());
        }
        let options = &self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?
            .options;
        let plan = PacketPlan::compile(&payload, options.mtu)?;
        self.reconcile_inner(connections, key, fresh_at, Some(plan))
            .await
    }

    /// Install the drop sink before saved ACK HTTP waits, even when a structurally
    /// valid bundle publishes invalid/withdrawn source intent. It authorizes no I/O.
    pub async fn prepare_dataplane_sink(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) -> Result<(), AgentError> {
        let result = self.prepare_dataplane_sink_inner(snapshot).await;
        if let Err(error) = &result {
            let reason = if matches!(error, AgentError::DataplaneResolveFailed) {
                Reason::ResolveFailed
            } else {
                Reason::InvalidConfiguration
            };
            // Preparation never authorizes renewal. Retain the owned route sink,
            // but close all old channels/packet tasks before returning to retry.
            self.withdraw(reason).await;
        }
        result
    }

    async fn prepare_dataplane_sink_inner(
        &mut self,
        snapshot: &meshway_protocol::AgentDataplaneSnapshot,
    ) -> Result<(), AgentError> {
        let settings = self
            .dataplane
            .as_ref()
            .ok_or(AgentError::Invalid("dataplane was not explicitly enabled"))?;
        let plan = PacketPlan::sink(&snapshot.payload, settings.options.mtu)?;
        for listener in &snapshot.payload.connections.listeners {
            let address: SocketAddr = listener
                .bind
                .parse()
                .map_err(|_| AgentError::Invalid("invalid transport listener"))?;
            if crate::managed_dataplane::in_prefix(address.ip(), plan.prefix) {
                return Err(AgentError::Invalid(
                    "transport listener overlaps mesh IPv4 pool",
                ));
            }
        }
        if self.tun.as_ref().is_some_and(|tun| !tun.matches(&plan)) {
            return Err(AgentError::Invalid("managed TUN binding changed"));
        }
        let targets = match time::timeout(
            Duration::from_secs(5),
            resolve_targets(&snapshot.payload.connections),
        )
        .await
        {
            Ok(Ok(targets)) => targets,
            Ok(Err(Failure(Reason::ResolveFailed))) | Err(_) => {
                return Err(AgentError::DataplaneResolveFailed);
            }
            Ok(Err(_)) => {
                return Err(AgentError::Invalid(
                    "invalid transport endpoint preparation",
                ));
            }
        };
        plan.check_targets(&targets)?;
        if self.tun.is_some() {
            return Ok(());
        }
        self.tun = Some(
            Tun::open(
                &settings.options,
                &plan,
                &settings.controller,
                settings.may_reuse_guard,
            )
            .await?,
        );
        Ok(())
    }

    pub fn observation(&self) -> Observation {
        self.running
            .as_ref()
            .map(|running| {
                let observed = running.observed.borrow().clone();
                if running.active() || observed.state != Phase::Applied {
                    observed
                } else {
                    let reason = match *running.deadline.borrow() {
                        None => Reason::ShuttingDown,
                        Some(deadline) if deadline <= Instant::now() => Reason::LeaseExpired,
                        _ => Reason::InternalError,
                    };
                    Observation::empty(Phase::Withdrawn, reason)
                }
            })
            .unwrap_or_else(|| self.observation.clone())
    }

    /// Per-generation watch channels are never shared with a replacement. This
    /// captures the actual proof snapshot and watch version in one read, fences
    /// the HTTP wait against later probe outcomes/withdrawal, and never renews.
    pub fn report_snapshot(&self) -> (Observation, Option<ReportGuard>) {
        if let Some(running) = &self.running
            && running.active()
        {
            let mut observed = running.observed.clone();
            let value = observed.borrow_and_update().clone();
            let deadline = running.deadline.subscribe();
            (
                value,
                Some(ReportGuard {
                    deadline,
                    observed,
                    observation_expiry: None,
                }),
            )
        } else {
            (self.observation(), None)
        }
    }

    /// Called immediately after persist/validation, before the saved ACK HTTP
    /// request can stall. Newly removed grants cannot survive that request.
    pub fn invalidate_if_changed(&self, snapshot: &AgentConnectionsSnapshot) {
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        if (!semantic.valid
            || self.semantic.as_ref().is_some_and(|old| old != &semantic)
            || (!self.discovery_options.servers.is_empty()
                && self.snapshot.as_ref().is_some_and(|old| {
                    old.payload.revision != snapshot.payload.revision
                        || old.digest != snapshot.digest
                })))
            && let Some(running) = &self.running
        {
            let _ = running.deadline.send(None);
        }
    }

    pub async fn withdraw(&mut self, reason: Reason) {
        #[cfg(unix)]
        self.probes.withdraw();
        if let Some(running) = self.running.take() {
            running.stop().await;
        }
        self.material = None;
        self.observation = Observation::empty(Phase::Withdrawn, reason);
    }

    pub fn validate_materials(
        &self,
        intent: &NodeConnections,
        materials: &[meshway_protocol::ManagedVlessMaterial],
    ) -> Result<(), AgentError> {
        profiles::load_combined(self.profiles.as_deref(), intent, materials)
            .map(|_| ())
            .map_err(|_| AgentError::Invalid("invalid managed carrier materials"))
    }

    pub fn install_materials(&mut self, materials: Vec<meshway_protocol::ManagedVlessMaterial>) {
        self.managed_materials = materials;
    }

    /// Only called after a fresh validated sync and repeated disk/binding/fsync
    /// checks under the Agent state lock. Reports never call this method.
    pub async fn reconcile(
        &mut self,
        snapshot: AgentConnectionsSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) {
        // This entry point always has allowed_paths=[], even in feature builds.
        let _ = self.reconcile_inner(snapshot, key, fresh_at, None).await;
    }

    async fn reconcile_inner(
        &mut self,
        snapshot: AgentConnectionsSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
        packet: Option<PacketPlan>,
    ) -> Result<(), AgentError> {
        let deadline = fresh_at + self.lease;
        let revision = snapshot.payload.revision;
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        if !self.discovery_options.servers.is_empty()
            && self.snapshot.as_ref().is_some_and(|old| {
                old.payload.revision != snapshot.payload.revision || old.digest != snapshot.digest
            })
        {
            self.withdraw(Reason::None).await;
        }
        self.snapshot = Some(snapshot);
        if deadline <= Instant::now() {
            self.withdraw(Reason::LeaseExpired).await;
            return Ok(());
        }
        if self.semantic.as_ref() != Some(&semantic) {
            self.withdraw(Reason::None).await;
            self.semantic = Some(semantic.clone());
        }
        if !semantic.valid {
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return Ok(());
        }
        let prepared = time::timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(5)),
            async {
                let profile_path = self.profiles.clone();
                let profile_intent = semantic.clone();
                let materials = self.managed_materials.clone();
                let profiles = tokio::task::spawn_blocking(move || {
                    profiles::load_combined(profile_path.as_deref(), &profile_intent, &materials)
                })
                .await
                .map_err(|_| Failure(Reason::InternalError))??;
                let targets = resolve_targets(&semantic).await?;
                Ok::<_, Failure>((profiles, targets))
            },
        )
        .await
        .unwrap_or(Err(Failure(Reason::ResolveFailed)));
        let (profiles, targets) = match prepared {
            Ok(value) => value,
            Err(Failure(reason)) => {
                self.withdraw(reason).await;
                self.observation.state = Phase::Error;
                return Ok(());
            }
        };
        let material = Material {
            profiles: profiles.fingerprint,
            targets: targets.clone(),
        };
        if let Some(plan) = &packet
            && let Err(error) = plan.check_targets(&targets)
        {
            self.withdraw(Reason::InvalidConfiguration).await;
            return Err(error);
        }
        if self.material.as_ref() == Some(&material)
            && self.running.as_ref().is_some_and(Running::active)
        {
            // Only completed, safe preparation may renew. An already-expired
            // generation cannot be revived by publication or a DNS retry.
            let _ = self
                .running
                .as_ref()
                .expect("active generation")
                .deadline
                .send(Some(deadline));
            #[cfg(unix)]
            self.probes.refresh_source_revision(revision);
            return Ok(());
        }
        self.withdraw(Reason::None).await;
        if let Some(plan) = &packet {
            if self.tun.as_ref().is_some_and(|tun| !tun.matches(plan)) {
                return Err(AgentError::Invalid(
                    "managed TUN address, prefix or MTU changed",
                ));
            }
            if self.tun.is_none() {
                let settings = self.dataplane.as_ref().expect("explicit dataplane options");
                self.tun = Some(
                    Tun::open(
                        &settings.options,
                        plan,
                        &settings.controller,
                        settings.may_reuse_guard,
                    )
                    .await?,
                );
            }
            // Flush the bounded kernel queue while no generation owns the fd.
            meshway_dataplane::packet_io::Device::drain(
                self.tun.as_ref().expect("configured TUN").device.as_ref(),
            )?;
        }
        if deadline <= Instant::now() {
            self.withdraw(Reason::LeaseExpired).await;
            return Ok(());
        }
        let result = time::timeout_at(
            deadline,
            session::Session::start_with_packet(
                &semantic,
                key,
                profiles,
                &targets,
                packet.as_ref(),
                self.socket_protection.clone(),
            ),
        )
        .await;
        match result {
            Ok(Ok(mut session)) => {
                if deadline <= Instant::now() {
                    session.shutdown().await;
                    self.observation = Observation::empty(Phase::Withdrawn, Reason::LeaseExpired);
                    return Ok(());
                }
                let initial = session.initial_observation(&semantic);
                let (observations, observed) = watch::channel(initial);
                let (renew, deadlines) = watch::channel(Some(deadline));
                let source = discovery::Source {
                    revision,
                    digest: self
                        .snapshot
                        .as_ref()
                        .expect("current source")
                        .digest
                        .clone(),
                };
                let enabled = !self.discovery_options.servers.is_empty();
                let (discovered, discovery) = watch::channel(discovery::Publication::new(
                    source.clone(),
                    if enabled {
                        meshway_protocol::EndpointDiscoveryState::Probing
                    } else {
                        meshway_protocol::EndpointDiscoveryState::Disabled
                    },
                ));
                let discovery_task = if enabled {
                    self.discovery_generation =
                        self.discovery_generation
                            .checked_add(1)
                            .ok_or(AgentError::Invalid(
                                "endpoint discovery generation exhausted",
                            ))?;
                    let runtime = std::sync::Arc::downgrade(&session.runtime);
                    let intent = semantic.clone();
                    let servers = self.discovery_options.servers.clone();
                    let generation = self.discovery_generation;
                    let deadlines = renew.subscribe();
                    let observed = observed.clone();
                    let timestamps = self.discovery_timestamps.clone();
                    Some(tokio::spawn(async move {
                        discovery::run(
                            discovery::Carrier {
                                runtime,
                                intent,
                                generation,
                                source,
                                timestamps,
                            },
                            servers,
                            deadlines,
                            observed,
                            discovered,
                        )
                        .await;
                    }))
                } else {
                    None
                };
                #[cfg(unix)]
                self.probes
                    .publish(&session.runtime, &semantic, &renew, &observed, revision);
                let packet_io = packet.map(|plan| {
                    let tun = self.tun.as_ref().expect("configured TUN");
                    (tun.device.clone(), tun.name.clone(), plan)
                });
                let task = tokio::spawn(session.run_with_packet(
                    semantic,
                    deadlines,
                    observations,
                    packet_io,
                ));
                self.running = Some(Running {
                    revision,
                    deadline: renew,
                    observed,
                    task,
                    discovery,
                    discovery_task,
                });
                self.material = Some(material);
            }
            error => {
                let reason = match error {
                    Ok(Err(Failure(reason))) => reason,
                    _ => Reason::LeaseExpired,
                };
                self.observation = Observation::empty(Phase::Error, reason);
            }
        }
        Ok(())
    }
}

fn transport(value: ConnectionTransport) -> TransportKind {
    match value {
        ConnectionTransport::NativeUdp => TransportKind::NativeUdp,
        ConnectionTransport::NativeTcp => TransportKind::NativeTcp,
        ConnectionTransport::Vless => TransportKind::Vless,
    }
}

fn protocol_transport(value: TransportKind) -> ConnectionTransport {
    match value {
        TransportKind::NativeUdp => ConnectionTransport::NativeUdp,
        TransportKind::NativeTcp => ConnectionTransport::NativeTcp,
        TransportKind::Vless => ConnectionTransport::Vless,
    }
}

fn allowed_peers(intent: &NodeConnections, listener: &str) -> Vec<String> {
    intent
        .incoming
        .iter()
        .filter(|approval| approval.listener_id == listener)
        .map(|approval| approval.peer_node_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn candidate_label(id: &str, address_index: usize) -> String {
    if address_index == 0 {
        id.to_owned()
    } else {
        format!("{id}:address:{address_index}")
    }
}

fn controller_candidate<'a>(
    intent: &'a NodeConnections,
    label: &str,
) -> Option<&'a meshway_protocol::OutgoingConnectionCandidate> {
    intent
        .outgoing
        .iter()
        .find(|candidate| (0..8).any(|index| candidate_label(&candidate.id, index) == label))
}

async fn resolve_targets(
    intent: &NodeConnections,
) -> Result<BTreeMap<String, Vec<SocketAddr>>, Failure> {
    stream::iter(intent.outgoing.iter().map(|candidate| async move {
        let resolved = time::timeout(
            Duration::from_secs(2),
            tokio::net::lookup_host((candidate.endpoint.host.as_str(), candidate.endpoint.port)),
        )
        .await
        .map_err(|_| Failure(Reason::ResolveFailed))?
        .map_err(|_| Failure(Reason::ResolveFailed))?;
        let mut addresses = BTreeSet::new();
        for address in resolved.take(32) {
            let ip = match address.ip() {
                std::net::IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .map(std::net::IpAddr::V4)
                    .unwrap_or(ip.into()),
                ip => ip,
            };
            if address.port() != 0
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
            {
                addresses.insert(SocketAddr::new(ip, address.port()));
            }
        }
        if addresses.is_empty() {
            return Err(Failure(Reason::ResolveFailed));
        }
        Ok((
            candidate.id.clone(),
            addresses.into_iter().take(8).collect(),
        ))
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect()
}

fn core_config(
    intent: &NodeConnections,
    private_key: PrivateKey,
    targets: &BTreeMap<String, Vec<SocketAddr>>,
) -> Result<LabConfig, Failure> {
    let trusted_peers = intent
        .peers
        .iter()
        .map(|peer| {
            let transports = intent
                .outgoing
                .iter()
                .filter(|item| item.peer_node_id == peer.node_id)
                .map(|item| transport(item.transport))
                .chain(
                    intent
                        .incoming
                        .iter()
                        .filter(|item| item.peer_node_id == peer.node_id)
                        .map(|item| transport(item.transport)),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            PeerConfig {
                node_id: peer.node_id.clone(),
                public_key: peer.public_key.clone(),
                transports,
            }
        })
        .collect();
    let listeners = intent
        .listeners
        .iter()
        .filter(|item| item.transport != ConnectionTransport::Vless)
        .map(|item| {
            Ok(ListenerConfig {
                transport: transport(item.transport),
                bind: item
                    .bind
                    .parse()
                    .map_err(|_| Failure(Reason::InvalidConfiguration))?,
                id: Some(item.id.clone()),
                allowed_peers: Some(allowed_peers(intent, &item.id)),
            })
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let mut outgoing = Vec::new();
    let mut native_targets = BTreeSet::new();
    // The validated projection is ordered by priority, then candidate ID. DNS
    // aliases can converge on the same physical target: keep the preferred
    // approved candidate's label/priority before enforcing core count limits.
    // Peer identity and carrier remain separate authorization boundaries.
    for item in intent
        .outgoing
        .iter()
        .filter(|item| item.transport != ConnectionTransport::Vless)
    {
        let addresses = targets
            .get(&item.id)
            .filter(|targets| !targets.is_empty())
            .ok_or(Failure(Reason::ResolveFailed))?;
        for (index, address) in addresses.iter().enumerate() {
            if !native_targets.insert((&item.peer_node_id, transport(item.transport), *address)) {
                continue;
            }
            outgoing.push(OutgoingCandidate {
                peer: item.peer_node_id.clone(),
                transport: transport(item.transport),
                endpoint: address.to_string(),
                priority: item.priority,
                id: Some(candidate_label(&item.id, index)),
            });
        }
    }
    let config = LabConfig {
        node_id: intent.node_id.clone(),
        private_key,
        public_key: intent
            .public_key
            .clone()
            .ok_or(Failure(Reason::InvalidConfiguration))?,
        trusted_peers,
        allowed_paths: Vec::new(),
        listeners,
        outgoing,
        limits: Default::default(),
    };
    config
        .validate()
        .map_err(|_| Failure(Reason::InvalidConfiguration))?;
    Ok(config)
}
