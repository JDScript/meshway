//! Explicitly enabled adjacency application. Never installs packet/path grants.

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

#[cfg(unix)]
mod probe;
mod profiles;
mod session;
#[cfg(unix)]
pub(crate) use probe::Prober;
#[cfg(all(test, unix))]
mod probe_tests;
#[cfg(test)]
mod tests;

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
    deadline: watch::Sender<Option<Instant>>,
    observed: watch::Receiver<Observation>,
    task: JoinHandle<()>,
}

pub(crate) struct ReportGuard {
    deadline: watch::Receiver<Option<Instant>>,
    observed: watch::Receiver<Observation>,
}

impl ReportGuard {
    pub fn current(&self) -> bool {
        self.deadline
            .borrow()
            .is_some_and(|deadline| deadline > Instant::now())
            && self.observed.borrow().state == Phase::Applied
            && !self.observed.has_changed().unwrap_or(true)
            && !self.deadline.has_changed().unwrap_or(true)
    }

    pub async fn changed(&mut self) {
        if !self.current() {
            return;
        }
        let deadline = (*self.deadline.borrow()).expect("checked active deadline");
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
    profiles: Option<PathBuf>,
    lease: Duration,
    running: Option<Running>,
    semantic: Option<NodeConnections>,
    material: Option<Material>,
    observation: Observation,
    pub snapshot: Option<AgentConnectionsSnapshot>,
    #[cfg(unix)]
    probes: probe::Publisher,
}

#[derive(PartialEq, Eq)]
struct Material {
    profiles: [u8; 32],
    targets: BTreeMap<String, Vec<SocketAddr>>,
}

impl Manager {
    pub fn new(profiles: Option<PathBuf>, lease: Duration) -> Self {
        Self {
            profiles,
            lease,
            running: None,
            semantic: None,
            material: None,
            observation: Observation::empty(Phase::Withdrawn, Reason::None),
            snapshot: None,
            #[cfg(unix)]
            probes: probe::Publisher::new(),
        }
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
            (value, Some(ReportGuard { deadline, observed }))
        } else {
            (self.observation(), None)
        }
    }

    /// Called immediately after persist/validation, before the saved ACK HTTP
    /// request can stall. Newly removed grants cannot survive that request.
    pub fn invalidate_if_changed(&self, snapshot: &AgentConnectionsSnapshot) {
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        if (!semantic.valid || self.semantic.as_ref().is_some_and(|old| old != &semantic))
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

    /// Only called after a fresh validated sync and repeated disk/binding/fsync
    /// checks under the Agent state lock. Reports never call this method.
    pub async fn reconcile(
        &mut self,
        snapshot: AgentConnectionsSnapshot,
        key: PrivateKey,
        fresh_at: Instant,
    ) {
        let deadline = fresh_at + self.lease;
        let mut semantic = snapshot.payload.clone();
        semantic.revision = 0;
        self.snapshot = Some(snapshot);
        if self.semantic.as_ref() != Some(&semantic) {
            self.withdraw(Reason::None).await;
            self.semantic = Some(semantic.clone());
        }
        if !semantic.valid {
            self.withdraw(Reason::InvalidConfiguration).await;
            self.observation.state = Phase::Error;
            return;
        }
        if let Some(running) = &self.running {
            // Renewal cannot revive an already-expired generation.
            if running.active() {
                let _ = running.deadline.send(Some(deadline));
            }
        }
        let prepared = time::timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(5)),
            async {
                let profile_path = self.profiles.clone();
                let profile_intent = semantic.clone();
                let profiles = tokio::task::spawn_blocking(move || {
                    profiles::load(profile_path.as_deref(), &profile_intent)
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
                return;
            }
        };
        let material = Material {
            profiles: profiles.fingerprint,
            targets: targets.clone(),
        };
        if self.material.as_ref() == Some(&material)
            && self.running.as_ref().is_some_and(Running::active)
        {
            return;
        }
        self.withdraw(Reason::None).await;
        let result = time::timeout_at(
            deadline,
            session::Session::start(&semantic, key, profiles, &targets),
        )
        .await;
        match result {
            Ok(Ok(session)) => {
                let initial = session.initial_observation(&semantic);
                let (observations, observed) = watch::channel(initial);
                let (renew, deadlines) = watch::channel(Some(deadline));
                #[cfg(unix)]
                self.probes
                    .publish(&session.runtime, &semantic, &renew, &observed);
                let task = tokio::spawn(session.run(semantic, deadlines, observations));
                self.running = Some(Running {
                    deadline: renew,
                    observed,
                    task,
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
