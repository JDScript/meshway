use crate::{
    Carrier, Error, Limits, SocketFactory, adjacency, provider,
    scope::{self, Scope},
};
use meshway_authority::DataPlaneIdentity;
use meshway_dataplane::secure::PendingFrame;
use meshway_model::{NodeId, observations::LinkAcquisition};
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

mod batch;
mod continuity;
mod datagram;
mod resources;
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
    if operation == "install" && cause == "authenticated_ready" {
        // Read the original admitted scope, never a newer shared publication or
        // current candidates. Failure to observe it drops diagnostics only.
        let Ok(cfg) = scope.snapshot.configuration() else {
            return;
        };
        let Some(peer) = cfg.peers.get(&fact.peer) else {
            return;
        };
        let Ok(observed_at_ms) = scope.snapshot.now_ms() else {
            return;
        };
        let instance = scope.snapshot.instance();
        let reference = scope.snapshot.reference();
        let (acquisition, attempt_id, endpoint_source) = match &fact.acquisition {
            LinkAcquisition::PublishedEndpoint => ("publishedEndpoint", String::new(), ""),
            LinkAcquisition::AcceptedInbound => ("acceptedInbound", String::new(), ""),
            LinkAcquisition::CoordinatedUdp {
                attempt_id,
                endpoint_source,
            } => (
                "coordinatedUdp",
                attempt_id.to_string(),
                match endpoint_source {
                    meshway_model::observations::EndpointKind::Configured => "configured",
                    meshway_model::observations::EndpointKind::Interface => "interface",
                    meshway_model::observations::EndpointKind::Stun => "stun",
                },
            ),
        };
        tracing::debug!(operation, cause, peer = %fact.peer, channel = ?fact.id,
            socket = ?fact.socket_id, dialer = %fact.carrier.dialer, listener = %fact.carrier.listener,
            listener_key = %fact.carrier.listener_key, protocol = ?fact.carrier.protocol,
            local = %fact.local, remote = %fact.remote,
            health_remaining_ms = ?health.map(|value| value.1), health_expired = ?health.map(|value| value.0),
            attempted_remote = ?attempted.map(|value| value.0), attempt = ?attempted.map(|value| value.1),
            publication = %reference.execution_publication, execution_expires_at_ms = scope.snapshot.expires_at_ms(),
            install_schema = 1_u64, install_observed_at_ms = observed_at_ms,
            install_node = %cfg.node_id, install_network = %cfg.network_id,
            install_instance = %instance.instance_id, install_instance_generation = instance.generation.get(),
            install_revision = reference.revision.get(), install_digest = %reference.digest,
            install_link = %meshway_model::Digest::from_bytes(fact.id),
            install_socket = %meshway_model::Digest::from_bytes(fact.socket_id),
            install_protocol = fact.carrier.protocol.as_str(),
            install_acquisition = acquisition, install_attempt_id = attempt_id, install_endpoint_source = endpoint_source,
            install_control_id = %cfg.self_keys.control_key.id,
            install_control_generation = cfg.self_keys.control_key.generation.get(),
            install_control_bytes = %cfg.self_keys.control_key.public_key.bytes,
            install_dataplane_id = %cfg.self_keys.dataplane_key.id,
            install_dataplane_generation = cfg.self_keys.dataplane_key.generation.get(),
            install_dataplane_bytes = %cfg.self_keys.dataplane_key.public_key.bytes,
            install_peer_control_id = %peer.control_key.id,
            install_peer_control_generation = peer.control_key.generation.get(),
            install_peer_control_bytes = %peer.control_key.public_key.bytes,
            install_peer_dataplane_id = %peer.dataplane_key.id,
            install_peer_dataplane_generation = peer.dataplane_key.generation.get(),
            install_peer_dataplane_bytes = %peer.dataplane_key.public_key.bytes,
            "Connectivity channel transition");
        return;
    }
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
    /// Acquisition of this authenticated channel, retained across compatible
    /// publication handoffs and independent of subsequent candidate expiry.
    pub acquisition: LinkAcquisition,
}

/// The actual selected channel and transport admission boundary of a locally
/// completed send. This does not assert remote receipt or authentication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendReceipt {
    pub link: LinkFact,
    pub admitted_at: std::time::Instant,
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
        /// Sampled when the formal receiver yields the complete frame, before
        /// event queueing or the consumer's cryptographic verification.
        received_at: std::time::Instant,
    },
    Closed,
}

struct Link {
    fact: LinkFact,
    sender: Arc<AsyncMutex<Sender>>,
    scope: Scope,
    batch_scope: meshway_transport::ChannelScope,
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
    socket_epochs: Arc<Mutex<BTreeMap<[u8; 32], meshway_authority::ExecutionSnapshot>>>,
    generation: meshway_model::Sequence,
    traversal_endpoints: Arc<Mutex<BTreeMap<[u8; 32], traversal::Endpoint>>>,
    resource_changes: tokio::sync::watch::Sender<()>,
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
    owners: BTreeMap<resources::Key, resources::Owner>,
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
            traversal_endpoints: Arc::new(Mutex::new(BTreeMap::new())),
            resource_changes: tokio::sync::watch::channel(()).0,
        });
        let mut manager = Self {
            factory,
            shared,
            listeners: Vec::new(),
            events,
            owners: BTreeMap::new(),
            traversal,
        };
        manager.install_resources().await?;
        if diagnostic_event() {
            tracing::debug!(operation = "start", publication = %manager.shared.identity.snapshot().reference().execution_publication,
                listeners = manager.listeners.len(), generation = manager.shared.generation.get(), "Connectivity manager transition");
        }
        Ok(manager)
    }

    async fn install_resources(&mut self) -> Result<(), Error> {
        let manager = self;
        let identity = manager.shared.identity.clone();
        let factory = manager.factory.clone();
        let limits = manager.shared.limits;
        let traversal = manager.traversal.clone();
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
            if manager
                .owners
                .contains_key(&resources::Key::Listener(key.clone()))
            {
                continue;
            }
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
                    if manager
                        .owners
                        .contains_key(&resources::Key::Candidate(key.clone(), ipv4))
                    {
                        continue;
                    }
                    if local.is_some_and(|local| local.is_ipv4() != ipv4) {
                        continue;
                    }
                    if traversal_endpoints
                        .iter()
                        .map(|(fact, _, _)| fact)
                        .chain(manager.listeners.iter())
                        .any(|fact| {
                            fact.local.is_ipv4() == ipv4
                                && traversal::listener_covers(
                                    &cfg.listeners[&fact.key],
                                    fact.local,
                                    options,
                                )
                        })
                    {
                        continue;
                    }
                    // Capacity is a startup error, never a permanently skipped
                    // suffix of configured peers. Partial bindings drop here.
                    if bound_candidates.len()
                        + manager
                            .owners
                            .keys()
                            .filter(|key| matches!(key, resources::Key::Candidate(..)))
                            .count()
                        >= limits.candidates
                    {
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
                    bound_candidates.push((
                        resources::Key::Candidate(key.clone(), ipv4),
                        id,
                        reference,
                        socket,
                    ));
                }
            }
        }
        let plans = workers::plans(&identity, limits)?;
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        for (fact, listener, acceptor) in prepared {
            manager.register_listener(&fact)?;
            manager.listeners.push(fact.clone());
            manager.spawn_owner(
                resources::Key::Listener(fact.key.clone()),
                Some(fact.socket_id),
                |shared| workers::listen(shared, fact, listener, acceptor),
            );
        }
        for (fact, endpoint) in datagrams {
            manager.register_listener(&fact)?;
            manager.listeners.push(fact.clone());
            manager.spawn_owner(
                resources::Key::Listener(fact.key.clone()),
                Some(fact.socket_id),
                |shared| datagram::listen(shared, fact, endpoint),
            );
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
            manager.spawn_owner(
                resources::Key::Listener(fact.key.clone()),
                Some(fact.socket_id),
                |shared| {
                    datagram::listen_traversal(shared, fact.socket_id, Some(fact), socket, received)
                },
            );
        }
        for (key, id, reference, socket) in bound_candidates {
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
            manager.spawn_owner(key, Some(id), |shared| {
                datagram::listen_traversal(shared, id, None, socket, received)
            });
        }
        let added_endpoints = !endpoints.is_empty();
        manager
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .extend(endpoints);
        if added_endpoints {
            manager.shared.resource_changes.send_replace(());
        }
        if !manager
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .is_empty()
            && !manager.owners.contains_key(&resources::Key::Traversal)
            && let Some((gateway, _)) = manager.traversal.clone()
        {
            manager.spawn_owner(resources::Key::Traversal, None, |shared| {
                traversal::run(shared, gateway)
            });
        }
        for peer in plans.into_keys() {
            if !manager.owners.contains_key(&resources::Key::Peer(peer)) {
                let factory = manager.factory.clone();
                manager.spawn_owner(resources::Key::Peer(peer), None, |shared| {
                    workers::dial(shared, factory, peer)
                });
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.snapshot");
        let _profile = PROFILE.enter();
        self.check_view()?;
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
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.carriers");
        let _profile = PROFILE.enter();
        self.check_view()?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        Ok(links
            .values()
            .filter(|link| link.scope.check().is_ok())
            .map(|link| (link.fact.peer, link.fact.carrier.clone()))
            .collect())
    }

    fn check_view(&self) -> Result<(), Error> {
        if self.shared.stop.is_cancelled() {
            return Err(Error::Unavailable);
        }
        self.shared
            .identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)
    }

    /// Never redirects or re-signs a PendingFrame. Exact signed carrier match is
    /// checked before queue admission and again after obtaining the send owner.
    pub async fn send(&self, pending: PendingFrame) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.send.poll");
        PROFILE
            .poll(self.send_with_receipt(pending))
            .await
            .map(|_| ())
    }

    /// Returns the channel captured at this call, with the formal sender's
    /// successful admission time. Channel replacement cancels the captured
    /// scope under the selection lock, including outputs waiting for admission
    /// or already accepted by the transport. The owned future never reselects
    /// a channel when first polled or borrows the Manager/receiver.
    pub fn send_with_receipt(
        &self,
        pending: PendingFrame,
    ) -> impl std::future::Future<Output = Result<SendReceipt, Error>> + Send + 'static + use<>
    {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.select");
        let _profile = PROFILE.enter();
        let shared = self.shared.clone();
        let selected = (|| -> Result<_, Error> {
            pending.ensure_current().map_err(|_| Error::Authority)?;
            if shared.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let links = shared.links.lock().map_err(|_| Error::Worker)?;
            let link = links.get(&pending.next()).ok_or(Error::Unavailable)?;
            if &link.fact.carrier != pending.carrier() {
                return Err(Error::Unavailable);
            }
            link.scope.check()?;
            Ok((link.sender.clone(), link.scope.clone(), link.fact.clone()))
        })();
        async move {
            pending.ensure_current().map_err(|_| Error::Authority)?;
            if shared.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let (sender, scope, fact) = selected?;
            scope
                .run(async {
                    scope.check()?;
                    let current = shared
                        .links
                        .lock()
                        .map_err(|_| Error::Worker)?
                        .get(&fact.peer)
                        .is_some_and(|link| link.fact.id == fact.id);
                    if !current {
                        return Err(Error::Unavailable);
                    }
                    let receipt = meshway_dataplane::carrier::send_shared_with_receipt(
                        &sender,
                        pending,
                        fact.remote,
                        Arc::new(scope.clone()),
                    )
                    .await?;
                    Ok(SendReceipt {
                        link: fact,
                        admitted_at: receipt.admitted_at,
                    })
                })
                .await
        }
    }

    pub async fn recv(&mut self) -> Event {
        self.receive_events(1).await.pop().expect("one event")
    }

    /// Drain at most 32 already queued events per wakeup. Each frame receives
    /// the same current-channel and execution checks as single-event receive.
    /// The returned facts are observations, not transferable I/O authority.
    pub async fn recv_batch(&mut self) -> Vec<Event> {
        self.receive_events(32).await
    }

    async fn receive_events(&mut self, limit: usize) -> Vec<Event> {
        loop {
            let mut events = Vec::new();
            let count = tokio::select! { biased;
                _ = self.shared.stop.cancelled() => return vec![Event::Closed],
                _ = self.shared.identity.snapshot().invalidated() => return vec![Event::Closed],
                count = self.events.recv_many(&mut events, limit) => count,
            };
            if count == 0
                || self.shared.stop.is_cancelled()
                || self.shared.identity.snapshot().ensure_current().is_err()
            {
                return vec![Event::Closed];
            }
            events.retain(|event| match event {
                Event::Frame { link, .. } => self.shared.links.lock().is_ok_and(|links| {
                    links.get(&link.peer).is_some_and(|current| {
                        current.fact.id == link.id && current.scope.check().is_ok()
                    })
                }),
                _ => true,
            });
            if self.shared.stop.is_cancelled()
                || self.shared.identity.snapshot().ensure_current().is_err()
            {
                return vec![Event::Closed];
            }
            if !events.is_empty() {
                return events;
            }
            tokio::task::yield_now().await;
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
        for (_, owner) in std::mem::take(&mut self.owners) {
            failed |= owner.join().await.is_err();
        }
        self.shared.links.lock().map_err(|_| Error::Worker)?.clear();
        self.listeners.clear();
        self.shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .clear();
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
            batch_scope: meshway_transport::ChannelScope::new(),
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
                let frames = receiver
                    .recv_batch()
                    .await
                    .map_err(|_| Error::Unavailable)?;
                let received_at = std::time::Instant::now();
                for frame in frames {
                    shared
                        .event(
                            &scope,
                            Event::Frame {
                                link: Box::new(fact.clone()),
                                frame,
                                received_at,
                            },
                        )
                        .await?;
                }
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
