use crate::{
    Carrier, Error, Limits, SocketFactory, adjacency, provider,
    scope::{self, Scope},
};
use meshway_control::DataPlaneIdentity;
use meshway_dataplane::secure::PendingFrame;
use meshway_model::NodeId;
use meshway_policy::admission::admit_adjacency;
use meshway_transport::{Frame, Sender, Session};
use meshway_transport_spec::TransportProtocol;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

mod continuity;
mod datagram;
#[cfg(test)]
mod tests;
mod traversal;
mod workers;

// Diagnostics only: at most 64 events/second and 32768/process; contention drops
// a diagnostic rather than delaying an owner or changing a selection decision.
fn diagnostic_event() -> bool {
    static BUDGET: std::sync::OnceLock<Mutex<(std::time::Instant, u32, u32)>> =
        std::sync::OnceLock::new();
    let Ok(mut budget) = BUDGET
        .get_or_init(|| Mutex::new((std::time::Instant::now(), 0, 0)))
        .try_lock()
    else {
        return false;
    };
    if budget.0.elapsed() >= std::time::Duration::from_secs(1) {
        budget.0 = std::time::Instant::now();
        budget.1 = 0;
    }
    if budget.1 >= 64 || budget.2 >= 32_768 {
        return false;
    }
    budget.1 += 1;
    budget.2 += 1;
    true
}

fn diagnostic_link(
    shared: &Shared,
    fact: &LinkFact,
    scope: &Scope,
    operation: &'static str,
    cause: &'static str,
    attempted: Option<(SocketAddr, [u8; 32])>,
) {
    if !diagnostic_event() {
        return;
    }
    let health = scope.health.as_ref().and_then(|health| {
        health.try_lock().ok().map(|deadline| {
            let now = tokio::time::Instant::now();
            (
                *deadline <= now,
                deadline.saturating_duration_since(now).as_millis() as u64,
            )
        })
    });
    tracing::debug!(operation, cause, peer = %fact.peer, channel = ?fact.id,
        socket = ?fact.socket_id, dialer = %fact.carrier.dialer, listener = %fact.carrier.listener,
        listener_key = %fact.carrier.listener_key, protocol = ?fact.carrier.protocol,
        local = %fact.local, remote = %fact.remote,
        health_remaining_ms = ?health.map(|value| value.1), health_expired = ?health.map(|value| value.0),
        attempted_remote = ?attempted.map(|value| value.0),
        attempt = ?attempted.map(|value| value.1),
        publication = %shared.identity.snapshot().reference().execution_publication,
        execution_expires_at_ms = shared.identity.snapshot().expires_at_ms(),
        "Connectivity channel transition");
}

#[derive(Clone, Debug)]
pub struct ListenerFact {
    pub socket_id: [u8; 32],
    pub key: String,
    pub protocol: TransportProtocol,
    pub local: SocketAddr,
}

/// Only produced after registered-key adjacency authentication (UDP also
/// confirms the final ACK). This is not an encryption or delivery assertion.
/// `id` is channel-specific; it is not a logical IP circuit identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFact {
    pub id: [u8; 32],
    /// Physical owner identity. Multiple UDP links share one endpoint ID.
    pub socket_id: [u8; 32],
    pub peer: NodeId,
    pub carrier: Carrier,
    pub local: SocketAddr,
    pub remote: SocketAddr,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub listeners: Vec<ListenerFact>,
    pub links: Vec<LinkFact>,
    pub sockets: Vec<SocketFact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketFact {
    pub reference: meshway_protocol::connectivity::SocketReference,
    pub network: meshway_model::state::SocketNetwork,
    pub bound_address: SocketAddr,
    pub endpoints: Vec<meshway_model::state::EndpointCandidate>,
}
impl SocketFact {
    pub fn candidate(
        &self,
        observation: &meshway_model::state::EndpointCandidate,
    ) -> Result<meshway_protocol::connectivity::CandidateReference, Error> {
        meshway_protocol::connectivity::CandidateReference::from_observation(
            self.reference.clone(),
            self.network,
            self.bound_address,
            observation,
        )
        .map_err(|_| Error::Configuration)
    }
}

#[derive(Debug)]
pub enum Event {
    /// Re-read actual current facts/carriers; no peer state report grants a link.
    Changed,
    Frame {
        link: Box<LinkFact>,
        frame: Frame,
    },
    Closed,
}

struct Link {
    fact: LinkFact,
    sender: Arc<AsyncMutex<Sender>>,
    scope: Scope,
}

/// The one selected-link owner also remembers its latest committed ID per peer.
/// This bounded metadata prevents an empty-slot ABA during passive recovery.
/// Handoff removes departed peers only after fencing old installation epochs;
/// peers still in the configuration retain their tombstones even while vacant.
#[derive(Default)]
struct LinkTable {
    current: BTreeMap<NodeId, Link>,
    last: BTreeMap<NodeId, [u8; 32]>,
}
impl std::ops::Deref for LinkTable {
    type Target = BTreeMap<NodeId, Link>;
    fn deref(&self) -> &Self::Target {
        &self.current
    }
}
impl std::ops::DerefMut for LinkTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.current
    }
}
impl LinkTable {
    fn insert(&mut self, peer: NodeId, link: Link) {
        self.last.insert(peer, link.fact.id);
        self.current.insert(peer, link);
    }
    fn clear(&mut self) {
        self.current.clear();
        self.last.clear();
    }
}

struct Shared {
    identity: DataPlaneIdentity,
    stop: CancellationToken,
    links: Arc<Mutex<LinkTable>>,
    events: mpsc::Sender<Event>,
    capacity: Arc<Semaphore>,
    limits: Limits,
    control_budget: Arc<Mutex<datagram::ControlBudget>>,
    sockets: Arc<Mutex<BTreeMap<[u8; 32], SocketFact>>>,
    epochs: tokio::sync::watch::Sender<DataPlaneIdentity>,
    gates: Arc<Mutex<Vec<std::sync::Weak<scope::Physical>>>>,
    socket_epochs: Arc<Mutex<BTreeMap<[u8; 32], meshway_control::ExecutionSnapshot>>>,
    generation: meshway_model::Sequence,
}

impl Shared {
    fn scope(&self, edge: Option<(NodeId, Carrier)>, handshake: bool) -> Scope {
        let deadline: tokio::time::Instant = self.identity.snapshot().expires_at().into();
        Scope {
            snapshot: self.identity.snapshot().clone(),
            cancellation: self.stop.child_token(),
            deadline: if handshake {
                deadline.min(tokio::time::Instant::now() + self.limits.handshake_timeout)
            } else {
                deadline
            },
            edge,
            health: None,
        }
    }

    async fn event(&self, scope: &Scope, event: Event) -> Result<(), Error> {
        scope
            .run(async { self.events.send(event).await.map_err(|_| Error::Cancelled) })
            .await
    }
}

/// Prepared listeners and bounded physical owners. A compatible direct Control
/// handoff replaces operation scopes while retaining authenticated channels;
/// incompatible resources use shutdown/replacement. The host keeps its unique
/// SecureNode/FlowTable for logical flow continuity.
pub struct Manager {
    factory: Arc<dyn SocketFactory>,
    shared: Arc<Shared>,
    listeners: Vec<ListenerFact>,
    events: mpsc::Receiver<Event>,
    workers: JoinSet<()>,
    traversal: Option<(
        Arc<dyn crate::traversal::TraversalGateway>,
        meshway_model::Sequence,
    )>,
}

impl Manager {
    pub async fn start(
        identity: DataPlaneIdentity,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
    ) -> Result<Self, Error> {
        Self::start_inner(identity, factory, limits, None).await
    }

    pub async fn start_with_traversal(
        identity: DataPlaneIdentity,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
        gateway: Arc<dyn crate::traversal::TraversalGateway>,
        generation: meshway_model::Sequence,
    ) -> Result<Self, Error> {
        if generation.get() == 0 {
            return Err(Error::Configuration);
        }
        Self::start_inner(identity, factory, limits, Some((gateway, generation))).await
    }

    async fn start_inner(
        identity: DataPlaneIdentity,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
        traversal: Option<(
            Arc<dyn crate::traversal::TraversalGateway>,
            meshway_model::Sequence,
        )>,
    ) -> Result<Self, Error> {
        limits.validate()?;
        let cfg = identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        validate_scale(cfg, limits)?;
        let mut prepared = Vec::new();
        let mut datagrams = Vec::new();
        let mut traversal_endpoints = Vec::new();
        let mut bound_candidates = Vec::new();
        // All declarations must validate and bind before any worker is installed.
        for (key, declaration) in &cfg.listeners {
            let (protocol, enabled) = provider::declaration(declaration);
            if !enabled {
                continue;
            }
            if key.is_empty() || key.len() > 128 {
                return Err(Error::Configuration);
            }
            if protocol == TransportProtocol::NativeUdp {
                if let Some((_, generation)) = &traversal {
                    let id = adjacency::nonce()?;
                    let reference = traversal::socket_reference(
                        &identity,
                        id,
                        *generation,
                        meshway_model::state::ResourceRef::Listener { key: key.clone() },
                    )?;
                    let socket = tokio::select! { biased;
                        _ = identity.snapshot().invalidated() => return Err(Error::Authority),
                        result = tokio::time::timeout(limits.handshake_timeout, factory.traversal_listener(identity.snapshot(), key, reference.clone())) => result.map_err(|_| Error::Cancelled)??,
                    };
                    let fact = ListenerFact {
                        socket_id: id,
                        key: key.clone(),
                        protocol,
                        local: socket.endpoint.local_addr()?,
                    };
                    traversal_endpoints.push((fact, reference, socket));
                    continue;
                }
                let endpoint = tokio::select! { biased;
                    _ = identity.snapshot().invalidated() => return Err(Error::Authority),
                    result = tokio::time::timeout(limits.handshake_timeout,
                        factory.datagram_listener(identity.snapshot(), key)) => result.map_err(|_| Error::Cancelled)??,
                };
                let fact = ListenerFact {
                    socket_id: adjacency::nonce()?,
                    key: key.clone(),
                    protocol,
                    local: endpoint.local_addr()?,
                };
                datagrams.push((fact, endpoint));
                continue;
            }
            let acceptor = Arc::new(provider::Acceptor::new(declaration)?);
            let listener = tokio::select! { biased;
                _ = identity.snapshot().invalidated() => return Err(Error::Authority),
                result = tokio::time::timeout(limits.handshake_timeout, factory.listen(identity.snapshot(), key)) => result.map_err(|_| Error::Cancelled)??,
            };
            let fact = ListenerFact {
                socket_id: adjacency::nonce()?,
                key: key.clone(),
                protocol,
                local: listener.local_addr()?,
            };
            prepared.push((fact, listener, acceptor));
        }
        if let Some((_, generation)) = &traversal {
            // Dial-only acquisition uses a real configured Native UDP socket,
            // even when the compiler intentionally retained no published target.
            for (key, candidate) in &cfg.dialers {
                let meshway_transport_spec::DialParameters::NativeUdp { options } =
                    &candidate.parameters
                else {
                    continue;
                };
                if !options.enabled || !options.bind_interface.is_empty() {
                    continue;
                }
                let local = if options.local_address.is_empty() {
                    None
                } else {
                    Some(
                        options
                            .local_address
                            .parse::<std::net::IpAddr>()
                            .map_err(|_| Error::Configuration)?,
                    )
                };
                for ipv4 in [true, false] {
                    if local.is_some_and(|local| local.is_ipv4() != ipv4) {
                        continue;
                    }
                    if traversal_endpoints.iter().any(|(fact, _, _)| {
                        fact.local.is_ipv4() == ipv4
                            && traversal::listener_covers(
                                &cfg.listeners[&fact.key],
                                fact.local,
                                options,
                            )
                    }) {
                        continue;
                    }
                    // Capacity is a startup error, never a permanently skipped
                    // suffix of configured peers. Partial bindings drop here.
                    if bound_candidates.len() >= limits.candidates {
                        return Err(Error::Capacity);
                    }
                    let id = adjacency::nonce()?;
                    let reference = traversal::socket_reference(
                        &identity,
                        id,
                        *generation,
                        meshway_model::state::ResourceRef::Dial { key: key.clone() },
                    )?;
                    let socket = tokio::select! { biased;
                        _ = identity.snapshot().invalidated() => return Err(Error::Authority),
                        result = tokio::time::timeout(limits.handshake_timeout, factory.bind_datagram_candidate(identity.snapshot(), key, ipv4, reference.clone())) => result.map_err(|_| Error::Cancelled)??,
                    };
                    bound_candidates.push((id, reference, socket));
                }
            }
        }
        let plans = workers::plans(&identity, limits)?;
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let (tx, events) = mpsc::channel(limits.events);
        let (epochs, _) = tokio::sync::watch::channel(identity.clone());
        let generation = traversal
            .as_ref()
            .map(|(_, generation)| *generation)
            .unwrap_or(
                meshway_model::Sequence::MIN
                    .checked_next()
                    .map_err(|_| Error::Capacity)?,
            );
        let shared = Arc::new(Shared {
            identity,
            stop: CancellationToken::new(),
            links: Arc::new(Mutex::new(LinkTable::default())),
            events: tx,
            capacity: Arc::new(Semaphore::new(limits.connections)),
            limits,
            control_budget: Arc::new(Mutex::new(datagram::ControlBudget::new())),
            sockets: Arc::new(Mutex::new(BTreeMap::new())),
            epochs,
            gates: Arc::new(Mutex::new(Vec::new())),
            socket_epochs: Arc::new(Mutex::new(BTreeMap::new())),
            generation,
        });
        let mut manager = Self {
            factory,
            shared,
            listeners: Vec::new(),
            events,
            workers: JoinSet::new(),
            traversal,
        };
        for (fact, listener, acceptor) in prepared {
            manager.register_listener(&fact)?;
            manager.listeners.push(fact.clone());
            manager.workers.spawn(workers::listen(
                manager.shared.clone(),
                fact,
                listener,
                acceptor,
            ));
        }
        for (fact, endpoint) in datagrams {
            manager.register_listener(&fact)?;
            manager.listeners.push(fact.clone());
            manager
                .workers
                .spawn(datagram::listen(manager.shared.clone(), fact, endpoint));
        }
        let mut endpoints = BTreeMap::new();
        for (fact, reference, socket) in traversal_endpoints {
            manager
                .shared
                .socket_epochs
                .lock()
                .map_err(|_| Error::Worker)?
                .insert(fact.socket_id, manager.shared.identity.snapshot().clone());
            manager.listeners.push(fact.clone());
            let (commands, received) = mpsc::channel(8);
            manager
                .shared
                .sockets
                .lock()
                .map_err(|_| Error::Worker)?
                .insert(
                    fact.socket_id,
                    SocketFact {
                        reference,
                        network: meshway_model::state::SocketNetwork::Udp,
                        bound_address: fact.local,
                        endpoints: Vec::new(),
                    },
                );
            endpoints.insert(
                fact.socket_id,
                traversal::Endpoint {
                    commands,
                    authorization: socket.authorization.clone(),
                },
            );
            manager.workers.spawn(datagram::listen_traversal(
                manager.shared.clone(),
                fact.socket_id,
                Some(fact),
                socket,
                received,
            ));
        }
        for (id, reference, socket) in bound_candidates {
            manager
                .shared
                .socket_epochs
                .lock()
                .map_err(|_| Error::Worker)?
                .insert(id, manager.shared.identity.snapshot().clone());
            let (commands, received) = mpsc::channel(8);
            manager
                .shared
                .sockets
                .lock()
                .map_err(|_| Error::Worker)?
                .insert(
                    id,
                    SocketFact {
                        reference,
                        network: meshway_model::state::SocketNetwork::Udp,
                        bound_address: socket.endpoint.local_addr()?,
                        endpoints: Vec::new(),
                    },
                );
            endpoints.insert(
                id,
                traversal::Endpoint {
                    commands,
                    authorization: socket.authorization.clone(),
                },
            );
            manager.workers.spawn(datagram::listen_traversal(
                manager.shared.clone(),
                id,
                None,
                socket,
                received,
            ));
        }
        if !endpoints.is_empty()
            && let Some((gateway, _)) = &manager.traversal
        {
            manager.workers.spawn(traversal::run(
                manager.shared.clone(),
                gateway.clone(),
                endpoints,
            ));
        }
        for (peer, candidates) in plans {
            manager.workers.spawn(workers::dial(
                manager.shared.clone(),
                manager.factory.clone(),
                peer,
                candidates,
            ));
        }
        if diagnostic_event() {
            tracing::debug!(operation = "start", publication = %manager.shared.identity.snapshot().reference().execution_publication,
                listeners = manager.listeners.len(), generation = manager.shared.generation.get(), "Connectivity manager transition");
        }
        Ok(manager)
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        if self.shared.stop.is_cancelled() {
            return Err(Error::Unavailable);
        }
        self.shared
            .identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        Ok(Snapshot {
            listeners: self.listeners.clone(),
            sockets: self
                .shared
                .sockets
                .lock()
                .map_err(|_| Error::Worker)?
                .values()
                .cloned()
                .collect(),
            links: links
                .values()
                .filter(|link| link.scope.check().is_ok())
                .map(|link| link.fact.clone())
                .collect(),
        })
    }

    fn register_listener(&self, fact: &ListenerFact) -> Result<(), Error> {
        self.shared
            .socket_epochs
            .lock()
            .map_err(|_| Error::Worker)?
            .insert(fact.socket_id, self.shared.identity.snapshot().clone());
        let generation = self
            .traversal
            .as_ref()
            .map(|(_, generation)| *generation)
            .unwrap_or(
                meshway_model::Sequence::MIN
                    .checked_next()
                    .map_err(|_| Error::Capacity)?,
            );
        let reference = traversal::socket_reference(
            &self.shared.identity,
            fact.socket_id,
            generation,
            meshway_model::state::ResourceRef::Listener {
                key: fact.key.clone(),
            },
        )?;
        self.shared
            .sockets
            .lock()
            .map_err(|_| Error::Worker)?
            .insert(
                fact.socket_id,
                SocketFact {
                    reference,
                    network: if fact.protocol == TransportProtocol::NativeUdp {
                        meshway_model::state::SocketNetwork::Udp
                    } else {
                        meshway_model::state::SocketNetwork::Tcp
                    },
                    bound_address: fact.local,
                    endpoints: Vec::new(),
                },
            );
        Ok(())
    }

    pub fn carriers(&self) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        Ok(self
            .snapshot()?
            .links
            .into_iter()
            .map(|link| (link.peer, link.carrier))
            .collect())
    }

    /// Never redirects or re-signs a PendingFrame. Exact signed carrier match is
    /// checked before queue admission and again after obtaining the send owner.
    pub async fn send(&self, pending: PendingFrame) -> Result<(), Error> {
        pending.ensure_current().map_err(|_| Error::Authority)?;
        let (sender, scope, fact) = {
            let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
            let link = links.get(&pending.next()).ok_or(Error::Unavailable)?;
            if &link.fact.carrier != pending.carrier() {
                return Err(Error::Unavailable);
            }
            (link.sender.clone(), link.scope.clone(), link.fact.clone())
        };
        scope
            .run(async {
                scope.check()?;
                let current = self
                    .shared
                    .links
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .get(&fact.peer)
                    .is_some_and(|link| link.fact.id == fact.id);
                if !current {
                    return Err(Error::Unavailable);
                }
                meshway_dataplane::carrier::send_shared(
                    &sender,
                    pending,
                    fact.remote,
                    Arc::new(scope.clone()),
                )
                .await?;
                Ok(())
            })
            .await
    }

    pub async fn recv(&mut self) -> Event {
        loop {
            let event = tokio::select! { biased;
                _ = self.shared.stop.cancelled() => return Event::Closed,
                _ = self.shared.identity.snapshot().invalidated() => return Event::Closed,
                _ = self.workers.join_next(), if !self.workers.is_empty() => {
                    self.shared.stop.cancel();
                    return Event::Closed;
                },
                event = self.events.recv() => match event { Some(event) => event, None => return Event::Closed },
            };
            if self.shared.stop.is_cancelled()
                || self.shared.identity.snapshot().ensure_current().is_err()
            {
                return Event::Closed;
            }
            if let Event::Frame { link, .. } = &event {
                let current = self.shared.links.lock().is_ok_and(|links| {
                    links.get(&link.peer).is_some_and(|current| {
                        current.fact.id == link.id && current.scope.check().is_ok()
                    })
                });
                if !current {
                    continue;
                }
            }
            return event;
        }
    }

    pub async fn reconcile(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        if self.can_reconcile(&identity) {
            return self.retain(identity).await;
        }
        // Fresh snapshot guards are mandatory even for otherwise identical targets.
        let traversal = self
            .traversal
            .as_ref()
            .map(|(gateway, generation)| {
                generation
                    .checked_next()
                    .map(|generation| (gateway.clone(), generation))
                    .map_err(|_| Error::Capacity)
            })
            .transpose()?;
        self.shutdown().await?;
        let replacement = Self::start_inner(
            identity,
            self.factory.clone(),
            self.shared.limits,
            traversal,
        )
        .await?;
        *self = replacement;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), Error> {
        if diagnostic_event() {
            tracing::debug!(operation = "shutdown", publication = %self.shared.identity.snapshot().reference().execution_publication,
                listeners = self.listeners.len(), "Connectivity manager transition");
        }
        if let Ok(links) = self.shared.links.lock() {
            for link in links.values() {
                diagnostic_link(
                    &self.shared,
                    &link.fact,
                    &link.scope,
                    "shutdown",
                    "manager_shutdown",
                    None,
                );
            }
        }
        self.shared.stop.cancel();
        let mut failed = false;
        while let Some(result) = self.workers.join_next().await {
            failed |= result.is_err();
        }
        self.shared.links.lock().map_err(|_| Error::Worker)?.clear();
        self.listeners.clear();
        self.events.close();
        while self.events.try_recv().is_ok() {}
        if failed { Err(Error::Worker) } else { Ok(()) }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.shared.stop.cancel();
        // JoinSet drop aborts tasks; every socket/session remains owned by a task
        // or this map. Explicit shutdown is the asynchronous join boundary.
        if let Ok(mut links) = self.shared.links.lock() {
            links.clear();
        }
    }
}

fn rank(fact: &LinkFact) -> (NodeId, NodeId, &str, TransportProtocol, [u8; 32]) {
    (
        fact.carrier.dialer,
        fact.carrier.listener,
        &fact.carrier.listener_key,
        fact.carrier.protocol,
        fact.id,
    )
}

fn validate_scale(
    cfg: &meshway_configuration::execution::NodeConfiguration,
    limits: Limits,
) -> Result<(), Error> {
    if cfg.listeners.len() > limits.listeners
        || cfg.dialers.len() > limits.candidates
        || cfg.peers.len() > limits.candidates
        || cfg
            .dialers
            .values()
            .try_fold(0usize, |count, dial| {
                count.checked_add(dial.endpoints.len())
            })
            .is_none_or(|count| count > limits.candidates)
    {
        return Err(Error::Capacity);
    }
    Ok(())
}

/// The only link selection/installation point, shared by both acquisition
/// drivers. Authentication has completed before constructing this value.
struct ReadyPeer {
    fact: LinkFact,
    sender: Arc<AsyncMutex<Sender>>,
    scope: Scope,
    /// Compare-and-replace condition captured during authenticated UDP
    /// reattachment. Never inferred from an address or unsolicited READY.
    supersedes: Replacement,
    attempt: Option<meshway_control::connectivity::AttemptScope>,
}

#[derive(Clone, Copy)]
enum Replacement {
    Ranked,
    Channel([u8; 32]),
    /// Recovery at a restarted listener must not overwrite a later winner.
    Vacant(Option<[u8; 32]>),
}

fn preferred_existing(old: &LinkFact, new: &LinkFact) -> bool {
    old.carrier.protocol != new.carrier.protocol
        || (old.carrier.protocol == TransportProtocol::NativeUdp && old.carrier == new.carrier)
        || rank(old) <= rank(new)
}

fn install_ready(shared: &Shared, ready: ReadyPeer) -> Result<bool, Error> {
    // A same-target renewal need not revoke the old Control snapshot. It must
    // nevertheless fence old acquisition commits before handoff can collect a
    // departed peer's ABA tombstone. Hold the watch read through installation:
    // publishing an epoch and committing a channel have one definite order.
    let epoch = shared.epochs.borrow();
    if !continuity::same(&ready.scope.snapshot, epoch.snapshot()) {
        ready.scope.cancellation.cancel();
        return Err(Error::Authority);
    }
    ready.scope.check()?;
    if let Some(attempt) = &ready.attempt {
        attempt.ensure_current().map_err(|_| Error::Authority)?;
    }
    let mut links = shared.links.lock().map_err(|_| Error::Worker)?;
    if !matches!(ready.supersedes, Replacement::Ranked) {
        let expected = match ready.supersedes {
            Replacement::Channel(id) => links.get(&ready.fact.peer).is_some_and(|old| {
                old.fact.id == id && old.fact.carrier.protocol == TransportProtocol::NativeUdp
            }),
            Replacement::Vacant(last) => {
                !links.contains_key(&ready.fact.peer)
                    && links.last.get(&ready.fact.peer).copied() == last
            }
            Replacement::Ranked => unreachable!(),
        };
        if ready.fact.carrier.protocol != TransportProtocol::NativeUdp || !expected {
            diagnostic_link(
                shared,
                &ready.fact,
                &ready.scope,
                "ready_rejected",
                "superseded_channel_changed",
                None,
            );
            ready.scope.cancellation.cancel();
            return Ok(false);
        }
    } else if let Some(old) = links.get(&ready.fact.peer)
        && old.scope.check().is_ok()
        && preferred_existing(&old.fact, &ready.fact)
    {
        diagnostic_link(
            shared,
            &old.fact,
            &old.scope,
            "ready_rejected",
            if old.fact.carrier.protocol != ready.fact.carrier.protocol {
                "existing_protocol"
            } else if old.fact.carrier.protocol == TransportProtocol::NativeUdp
                && old.fact.carrier == ready.fact.carrier
            {
                "healthy_same_udp_carrier"
            } else {
                "existing_rank"
            },
            Some((ready.fact.remote, ready.fact.id)),
        );
        ready.scope.cancellation.cancel();
        return Ok(false);
    }
    // A blocked lock/diagnostic must not let a cancelled handoff commit.
    let fact = ready.fact.clone();
    let scope = ready.scope.clone();
    ready.scope.check()?;
    if let Some(attempt) = &ready.attempt {
        attempt.ensure_current().map_err(|_| Error::Authority)?;
    }
    let old = links.remove(&ready.fact.peer);
    if let Some(old) = &old {
        old.scope.cancellation.cancel();
    }
    links.insert(
        ready.fact.peer,
        Link {
            fact: ready.fact,
            sender: ready.sender,
            scope: ready.scope,
        },
    );
    drop(links);
    if let Some(old) = old {
        diagnostic_link(
            shared,
            &old.fact,
            &old.scope,
            "retire",
            "selected_replacement",
            None,
        );
    }
    diagnostic_link(
        shared,
        &fact,
        &scope,
        "install",
        "authenticated_ready",
        None,
    );
    Ok(true)
}

fn retire(shared: &Shared, fact: &LinkFact) -> Result<(), Error> {
    let mut links = shared.links.lock().map_err(|_| Error::Worker)?;
    if links
        .get(&fact.peer)
        .is_some_and(|link| link.fact.id == fact.id)
        && let Some(link) = links.remove(&fact.peer)
    {
        diagnostic_link(
            shared,
            &link.fact,
            &link.scope,
            "retire",
            "owner_retire",
            None,
        );
        link.scope.cancellation.cancel();
    }
    Ok(())
}

async fn connected(
    mut shared: Arc<Shared>,
    stream: Box<dyn meshway_transport::raw::Stream>,
    fact: LinkFact,
    opening: Arc<scope::Opening>,
) -> Result<(), Error> {
    let mut updates = shared.epochs.subscribe();
    let mut scope = shared.scope(Some((fact.peer, fact.carrier.clone())), false);
    scope.check()?;
    let (sender, mut receiver) = Session::stream(
        scope::Stream {
            inner: stream,
            scope: shared.physical()?,
            opening: None,
        },
        fact.remote,
        shared.limits.session,
    )
    .into_split();
    let sender = Arc::new(AsyncMutex::new(sender));
    shared.register_link_socket(&fact)?;
    struct Registration(Arc<Shared>, [u8; 32]);
    impl Drop for Registration {
        fn drop(&mut self) {
            self.0.remove_socket(self.1);
        }
    }
    let _registration = Registration(shared.clone(), fact.socket_id);
    let selected = install_ready(
        &shared,
        ReadyPeer {
            fact: fact.clone(),
            sender: sender.clone(),
            scope: scope.clone(),
            supersedes: Replacement::Ranked,
            attempt: None,
        },
    )?;
    if !selected {
        sender
            .lock()
            .await
            .shutdown()
            .await
            .map_err(|_| Error::Worker)?;
        return Ok(());
    }
    if let Err(error) = opening.release() {
        scope.cancellation.cancel();
        retire(&shared, &fact)?;
        let _ = sender.lock().await.shutdown().await;
        return Err(error);
    }
    let result = loop {
        let receive = async {
            shared.event(&scope, Event::Changed).await?;
            loop {
                scope.check()?;
                let frame = receiver.recv().await.map_err(|_| Error::Unavailable)?;
                shared
                    .event(
                        &scope,
                        Event::Frame {
                            link: Box::new(fact.clone()),
                            frame,
                        },
                    )
                    .await?;
            }
        };
        let result = tokio::select! { biased;
            _ = updates.changed() => Err(Error::Authority),
            result = scope.run(receive) => result,
        };
        if shared.stop.is_cancelled() {
            break result;
        }
        if scope.snapshot.ensure_current().is_err() || shared.epoch_changed() {
            match shared.successor(&mut updates).await {
                Ok(next) => match next.renew_link(&fact)? {
                    Some(next_scope) => {
                        shared = next;
                        scope = next_scope;
                        continue;
                    }
                    None => break Err(Error::Unavailable),
                },
                Err(error) => break Err(error),
            }
        }
        break result;
    };
    scope.cancellation.cancel();
    drop(receiver);
    retire(&shared, &fact)?;
    sender
        .lock()
        .await
        .shutdown()
        .await
        .map_err(|_| Error::Worker)?;
    let _ = shared
        .event(&shared.scope(None, false), Event::Changed)
        .await;
    result
}
