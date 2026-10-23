use crate::{
    Carrier, Error, Limits, SocketFactory, adjacency,
    plan::Plan,
    provider,
    scope::{self, Scope},
};
use meshway_dataplane::carrier::SharedSender;
use meshway_dataplane::secure::PendingFrame;
use meshway_engine::DataPlaneKeys;
use meshway_engine::Desired;
use meshway_model::TransportProtocol;
use meshway_model::{NodeId, observations::LinkAcquisition};
use meshway_transport::{Frame, Session};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

mod apply;
mod batch;
mod datagram;
mod events;
mod evidence;
mod facts;
mod interest;
mod probe;
mod rebind;
mod resources;
mod roam;
#[cfg(test)]
mod tests;
mod traversal;
mod workers;
pub use interest::{AdjacencyInterest, Interest};
pub use probe::{SizeProbe, SocketObservation, StunProbe, StunReport, StunSample};
pub use rebind::ListenerUnavailable;
pub use roam::Roamed;

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
    if operation == "install"
        && cause == "authenticated_ready"
        && let Some(installation) = evidence::installed(fact, shared)
    {
        tracing::debug!(
            operation,
            cause,
            install_schema = 3_u64,
            install_json = installation,
            "Connectivity channel transition"
        );
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
    let netmap = shared.plan().reference;
    tracing::debug!(operation, cause, peer = %fact.peer, channel = ?fact.id,
        socket = ?fact.socket_id, dialer = %fact.carrier.dialer, listener = %fact.carrier.listener,
        listener_key = %fact.carrier.listener_key, protocol = ?fact.carrier.protocol,
        local = %fact.local, remote = %fact.remote,
        health_remaining_ms = ?health.map(|value| value.1), health_expired = ?health.map(|value| value.0),
        attempted_remote = ?attempted.map(|value| value.0), attempt = ?attempted.map(|value| value.1),
        netmap_serial = netmap.serial.get(),
        netmap_digest = %netmap.digest,
        acquisition = ?fact.acquisition,
        "Connectivity channel transition");
}

/// The one log line of a Mesh UDP socket's granted buffers. Linux reports
/// twice the granted bytes; less than requested means the kernel limited an
/// unprivileged request to `net.core.rmem_max`／`wmem_max`.
pub(super) fn log_buffers(
    resource: &str,
    local: SocketAddr,
    buffers: Option<meshway_transport::raw::Buffers>,
) {
    let Some(buffers) = buffers else {
        return;
    };
    if buffers.limited() {
        tracing::warn!(resource, %local,
            receive_buffer = buffers.receive, receive_requested = buffers.receive_requested,
            send_buffer = buffers.send, send_requested = buffers.send_requested,
            "UDP socket buffers limited by net.core.rmem_max/wmem_max");
    } else {
        tracing::info!(resource, %local,
            receive_buffer = buffers.receive, receive_requested = buffers.receive_requested,
            send_buffer = buffers.send, send_requested = buffers.send_requested,
            "UDP socket buffers");
    }
}

#[derive(Clone, Debug)]
pub struct ListenerFact {
    pub socket_id: [u8; 32],
    pub key: String,
    pub protocol: TransportProtocol,
    pub local: SocketAddr,
    /// Buffer sizes the kernel granted a datagram listener's socket.
    pub buffers: Option<meshway_transport::raw::Buffers>,
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
    /// Acquisition of this authenticated channel, kept across compatible
    /// content changes and independent of subsequent candidate expiry.
    pub acquisition: LinkAcquisition,
    /// The peer's signed boot identity in this channel's transcript. A new
    /// channel with the same boot is a carrier change of the same peer state;
    /// a different boot means the peer restarted and lost its circuits.
    pub peer_boot: crate::Boot,
}

/// Size limits of one selected Native UDP channel. `path` is the outer path
/// MTU its socket's route toward the remote knows (a smaller PMTU once ICMP
/// taught it); every outer datagram carries DF. `inner` is the largest inner
/// IP packet one Data frame carries without exceeding it: `path` minus the
/// outer IP/UDP header (28 IPv4, 48 IPv6) and the fixed Data frame overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkMtu {
    pub path: u16,
    pub inner: u16,
}
impl LinkMtu {
    pub fn new(path: u16, ipv4: bool) -> Option<Self> {
        let outer = if ipv4 { 28 } else { 48 } + meshway_dataplane::secure::DATA_FRAME_OVERHEAD;
        let inner = usize::from(path).checked_sub(outer)?;
        Some(Self {
            path,
            inner: u16::try_from(inner).ok()?,
        })
    }
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
    /// Admitted peers without a current channel: idle, or wanted but unreached.
    pub adjacencies: BTreeMap<NodeId, meshway_model::state::AdjacencyStatus>,
    /// Size limits of selected Native UDP channels whose route MTU is known.
    pub mtu: BTreeMap<NodeId, LinkMtu>,
    /// Configured listeners whose bind is being retried.
    pub unavailable: Vec<ListenerUnavailable>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketFact {
    pub reference: meshway_protocol::connectivity::SocketReference,
    pub network: meshway_model::state::SocketNetwork,
    pub bound_address: SocketAddr,
    pub endpoints: Vec<meshway_model::state::EndpointCandidate>,
    /// Buffer sizes the kernel granted this UDP socket; `None` for stream
    /// sockets and sockets whose provider reports none.
    pub buffers: Option<meshway_transport::raw::Buffers>,
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
    sender: Arc<SharedSender>,
    scope: Scope,
    batch_scope: meshway_transport::ChannelScope,
    activity: Arc<interest::Activity>,
}

/// The one selected-link owner also remembers its latest committed ID per peer.
/// This bounded metadata prevents an empty-slot ABA during passive recovery.
/// Apply removes departed peers' entries; peers still in the content retain
/// their tombstones even while vacant.
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

/// Mechanism state shared by the Manager and its owners. `plans` holds the
/// current desired content; owners read it when they act, and only the
/// Manager's apply replaces it. Each owner has its own child `stop`.
/// Route MTU by link ID, with the socket owning that channel.
type PathMtus = BTreeMap<[u8; 32], ([u8; 32], u16)>;

struct Shared {
    keys: DataPlaneKeys,
    plans: tokio::sync::watch::Sender<Arc<Plan>>,
    stop: CancellationToken,
    links: Arc<Mutex<LinkTable>>,
    events: events::Sender,
    control_events: events::Sender,
    capacity: Arc<Semaphore>,
    limits: Limits,
    control_budget: Arc<Mutex<datagram::ControlBudget>>,
    sockets: Arc<Mutex<BTreeMap<[u8; 32], SocketFact>>>,
    /// Last validated STUN response of each socket, whether or not it changed
    /// the reported mapping: the clock of its observation cadence.
    stun_confirmed: Arc<Mutex<BTreeMap<[u8; 32], tokio::time::Instant>>>,
    /// Latest validated STUN answer of each socket per observer address.
    stun_samples: Arc<Mutex<probe::StunSamples>>,
    /// Outer route MTU of each Native UDP channel by link ID, with the
    /// socket owning it; that owner keeps only its own channels.
    path_mtu: Arc<Mutex<PathMtus>>,
    /// Latest DF size probe toward each observer.
    size_probes: Arc<Mutex<BTreeMap<SocketAddr, probe::SizeProbe>>>,
    /// A local diagnostic asked every traversal socket to observe now.
    observation_request: Arc<Mutex<Option<tokio::time::Instant>>>,
    /// State socket generation of this Manager's descriptors.
    generation: meshway_model::Sequence,
    traversal_endpoints: Arc<Mutex<BTreeMap<[u8; 32], traversal::Endpoint>>>,
    resource_changes: tokio::sync::watch::Sender<()>,
    interests: Arc<interest::Interests>,
    facts: Arc<facts::Store>,
    /// Instance qualifying State socket references.
    instance: Arc<Mutex<Option<meshway_protocol::InstanceRequest>>>,
    /// This owner's boot identity, signed into every adjacency nonce.
    boot: crate::Boot,
    /// Listeners of the current content whose bind is being retried.
    unbound: Arc<rebind::Unbounds>,
    /// Underlay path changes this Manager reacted to ([`Manager::path_changed`]):
    /// every datagram owner probes its channels at once, and candidate
    /// pacing starts over.
    roams: Arc<tokio::sync::watch::Sender<u64>>,
    /// A path change asked every traversal socket to observe its mapping now.
    roam_observation: Arc<Mutex<Option<tokio::time::Instant>>>,
}

impl Shared {
    /// The current desired adjacency input.
    fn plan(&self) -> Arc<Plan> {
        self.plans.borrow().clone()
    }

    /// The same mechanism state under a child stop token of its own.
    fn for_owner(&self) -> Arc<Self> {
        Arc::new(Self {
            keys: self.keys.clone(),
            plans: self.plans.clone(),
            stop: self.stop.child_token(),
            links: self.links.clone(),
            events: self.events.clone(),
            control_events: self.control_events.clone(),
            capacity: self.capacity.clone(),
            limits: self.limits,
            control_budget: self.control_budget.clone(),
            sockets: self.sockets.clone(),
            stun_confirmed: self.stun_confirmed.clone(),
            stun_samples: self.stun_samples.clone(),
            path_mtu: self.path_mtu.clone(),
            size_probes: self.size_probes.clone(),
            observation_request: self.observation_request.clone(),
            generation: self.generation,
            traversal_endpoints: self.traversal_endpoints.clone(),
            resource_changes: self.resource_changes.clone(),
            interests: self.interests.clone(),
            facts: self.facts.clone(),
            instance: self.instance.clone(),
            boot: self.boot,
            unbound: self.unbound.clone(),
            roams: self.roams.clone(),
            roam_observation: self.roam_observation.clone(),
        })
    }

    /// Underlay path changes observed so far.
    fn roam_epoch(&self) -> u64 {
        *self.roams.borrow()
    }

    fn physical(&self) -> Result<Arc<scope::Physical>, Error> {
        Ok(Arc::new(scope::Physical::new(self.stop.clone())))
    }

    fn scope(&self, edge: Option<(NodeId, Carrier)>, handshake: bool) -> Scope {
        Scope {
            cancellation: self.stop.child_token(),
            deadline: handshake
                .then(|| tokio::time::Instant::now() + self.limits.handshake_timeout),
            edge,
            health: None,
        }
    }

    async fn event(&self, scope: &Scope, event: Event) -> Result<(), Error> {
        let sender = self.event_sender(&event);
        scope
            .run(async { sender.send(event).await.map_err(|_| Error::Cancelled) })
            .await
    }

    fn event_sender(&self, event: &Event) -> &events::Sender {
        if Self::business_event(event) {
            &self.events
        } else {
            &self.control_events
        }
    }

    fn business_event(event: &Event) -> bool {
        match event {
            // Scheduling hint only. SecureNode still authenticates and admits it.
            Event::Frame { frame, .. }
                if !meshway_dataplane::secure::is_control_frame(&frame.payload) =>
            {
                true
            }
            _ => false,
        }
    }
}

/// Prepared listeners and bounded physical owners for Engine's desired
/// content. [`Manager::apply`] keeps every resource and selected channel the
/// new content still admits and retires the rest; withdrawal is shutdown.
pub struct Manager {
    factory: Arc<dyn SocketFactory>,
    shared: Arc<Shared>,
    listeners: Vec<ListenerFact>,
    events: events::Receiver,
    control_events: events::Receiver,
    owners: BTreeMap<resources::Key, resources::Owner>,
    traversal: Option<(
        Arc<dyn crate::traversal::TraversalGateway>,
        meshway_model::Sequence,
    )>,
    /// Listeners bound by their retry owners, for [`Self::adopt`].
    rebound: mpsc::Receiver<rebind::Rebound>,
    rebound_sender: mpsc::Sender<rebind::Rebound>,
    rebind_tokens: u64,
}

impl Manager {
    pub async fn start(
        desired: &Desired,
        keys: DataPlaneKeys,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
    ) -> Result<Self, Error> {
        Self::start_inner(desired, keys, factory, limits, None, None).await
    }

    /// With coordinated UDP acquisition through the host's Control gateway.
    /// `instance` qualifies State socket references from the first bind, so
    /// Native UDP listeners are coordination sockets whenever one is live.
    /// `boot`: the boot of the Manager this one replaces while the packet
    /// owner keeps its circuits (peers must not see a restart); `None` for a
    /// new one.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_with_traversal(
        desired: &Desired,
        keys: DataPlaneKeys,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
        gateway: Arc<dyn crate::traversal::TraversalGateway>,
        generation: meshway_model::Sequence,
        instance: Option<meshway_protocol::InstanceRequest>,
        boot: Option<crate::Boot>,
    ) -> Result<Self, Error> {
        if generation.get() == 0 {
            return Err(Error::Configuration);
        }
        Self::start_inner(
            desired,
            keys,
            factory,
            limits,
            Some((gateway, generation)),
            Some(Carried {
                instance,
                boot,
                ..Default::default()
            }),
        )
        .await
    }

    async fn start_inner(
        desired: &Desired,
        keys: DataPlaneKeys,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
        traversal: Option<(
            Arc<dyn crate::traversal::TraversalGateway>,
            meshway_model::Sequence,
        )>,
        carried: Option<Carried>,
    ) -> Result<Self, Error> {
        limits.validate()?;
        let plan = Plan::new(desired)?;
        if !keys.matches(&plan.self_keys) {
            return Err(Error::Authentication);
        }
        // More small-packet slots absorb scheduling bursts without expanding
        // the former 64-frame maximum storage exposure (8 MiB by default).
        let frame = limits.session.max_frame_bytes();
        let bytes = (frame * limits.events.min(64))
            .min(8 * 1024 * 1024)
            .max(frame);
        let (tx, events) = events::channel(limits.events, bytes);
        let (control_tx, control_events) = events::channel(32, frame * 32);
        let generation = traversal
            .as_ref()
            .map(|(_, generation)| *generation)
            .unwrap_or(
                meshway_model::Sequence::MIN
                    .checked_next()
                    .map_err(|_| Error::Capacity)?,
            );
        let Carried {
            interests,
            facts,
            instance,
            boot,
        } = carried.unwrap_or_default();
        let boot = match boot {
            Some(boot) => boot,
            None => crate::adjacency::boot()?,
        };
        interests.allow(&plan);
        let shared = Arc::new(Shared {
            keys,
            plans: tokio::sync::watch::channel(plan).0,
            stop: CancellationToken::new(),
            links: Arc::new(Mutex::new(LinkTable::default())),
            events: tx,
            control_events: control_tx,
            capacity: Arc::new(Semaphore::new(limits.connections)),
            limits,
            control_budget: Arc::new(Mutex::new(datagram::ControlBudget::new())),
            sockets: Arc::new(Mutex::new(BTreeMap::new())),
            stun_confirmed: Arc::default(),
            stun_samples: Arc::default(),
            path_mtu: Arc::default(),
            size_probes: Arc::default(),
            observation_request: Arc::default(),
            generation,
            traversal_endpoints: Arc::new(Mutex::new(BTreeMap::new())),
            resource_changes: tokio::sync::watch::channel(()).0,
            interests,
            facts,
            instance: Arc::new(Mutex::new(instance)),
            boot,
            unbound: Arc::default(),
            roams: Arc::new(tokio::sync::watch::channel(0).0),
            roam_observation: Arc::default(),
        });
        let (rebound_sender, rebound) = mpsc::channel(limits.listeners);
        let mut manager = Self {
            factory,
            shared,
            listeners: Vec::new(),
            events,
            control_events,
            owners: BTreeMap::new(),
            traversal,
            rebound,
            rebound_sender,
            rebind_tokens: 0,
        };
        manager.install_resources().await?;
        if diagnostic_event() {
            tracing::debug!(
                operation = "start",
                netmap_serial = manager.shared.plan().reference.serial.get(),
                listeners = manager.listeners.len(),
                generation = manager.shared.generation.get(),
                "Connectivity manager transition"
            );
        }
        Ok(manager)
    }

    /// Handle for the packet owner to request on-demand adjacencies.
    pub fn interest(&self) -> Interest {
        Interest(self.shared.interests.clone())
    }

    async fn install_resources(&mut self) -> Result<(), Error> {
        let manager = self;
        let factory = manager.factory.clone();
        let limits = manager.shared.limits;
        let traversal = manager.traversal.clone();
        let plan = manager.shared.plan();
        validate_scale(&plan, limits)?;
        let mut prepared = Vec::new();
        let mut unbound = Vec::new();
        let mut bound_candidates = Vec::new();
        // Every declaration validates and binds before any worker is
        // installed; a listener whose address is unavailable is retried by
        // its own owner instead of withholding the rest.
        for (key, declaration) in &plan.listeners {
            if manager
                .owners
                .contains_key(&resources::Key::Listener(key.clone()))
            {
                continue;
            }
            let (_, enabled) = provider::declaration(declaration);
            if !enabled {
                continue;
            }
            if key.is_empty() || key.len() > 128 {
                return Err(Error::Configuration);
            }
            match rebind::bind(&manager.shared, &factory, traversal.is_some(), key).await {
                Ok(bound) => prepared.push(bound),
                Err(error @ Error::Bind { .. }) => unbound.push((key.clone(), error)),
                Err(error) => return Err(error),
            }
        }
        let bound_listeners: Vec<_> = prepared
            .iter()
            .filter_map(|bound| match bound {
                rebind::Bound::Traversal { fact, .. } => Some(fact.clone()),
                _ => None,
            })
            .collect();
        if traversal.is_some() {
            // Dial-only acquisition uses a real configured Native UDP socket,
            // even when the compiler intentionally kept no published target.
            for (key, candidate) in &plan.dialers {
                let meshway_transport::spec::DialParameters::NativeUdp { options } =
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
                    if bound_listeners
                        .iter()
                        .chain(manager.listeners.iter())
                        .any(|fact| {
                            fact.local.is_ipv4() == ipv4
                                && traversal::listener_covers(
                                    &plan.listeners[&fact.key],
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
                    let Some(reference) = traversal::socket_reference(
                        &manager.shared,
                        id,
                        meshway_model::state::ResourceRef::Dial { key: key.clone() },
                    )?
                    else {
                        continue;
                    };
                    let socket = tokio::time::timeout(
                        limits.handshake_timeout,
                        factory.bind_datagram_candidate(
                            &plan.desired,
                            key,
                            ipv4,
                            reference.clone(),
                        ),
                    )
                    .await
                    .map_err(|_| Error::Cancelled)??;
                    log_buffers(
                        &format!("dial {key}"),
                        socket.endpoint.local_addr()?,
                        socket.endpoint.buffers(),
                    );
                    bound_candidates.push((
                        resources::Key::Candidate(key.clone(), ipv4),
                        id,
                        reference,
                        socket,
                    ));
                }
            }
        }
        if manager.shared.stop.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut endpoints = BTreeMap::new();
        manager.install_bound(prepared, &mut endpoints)?;
        for (key, error) in unbound {
            manager.retry_listener(key, &error)?;
        }
        for (key, id, reference, socket) in bound_candidates {
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
                        buffers: socket.endpoint.buffers(),
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
        manager.publish_endpoints(endpoints)?;
        if !manager.owners.contains_key(&resources::Key::Idle) {
            manager.spawn_owner(resources::Key::Idle, None, interest::idle);
        }
        if !manager.owners.contains_key(&resources::Key::Facts) {
            manager.spawn_owner(resources::Key::Facts, None, facts::run);
        }
        for peer in plan.dial_peers() {
            if !manager.owners.contains_key(&resources::Key::Peer(peer)) {
                let factory = manager.factory.clone();
                manager.spawn_owner(resources::Key::Peer(peer), None, |shared| {
                    workers::dial(shared, factory, peer)
                });
            }
        }
        Ok(())
    }

    /// Serve bound listeners. Coordination sockets join `endpoints`.
    fn install_bound(
        &mut self,
        bound: Vec<rebind::Bound>,
        endpoints: &mut BTreeMap<[u8; 32], traversal::Endpoint>,
    ) -> Result<(), Error> {
        for bound in bound {
            match bound {
                rebind::Bound::Stream {
                    fact,
                    listener,
                    acceptor,
                } => {
                    self.register_listener(&fact)?;
                    self.listeners.push(fact.clone());
                    self.spawn_owner(
                        resources::Key::Listener(fact.key.clone()),
                        Some(fact.socket_id),
                        |shared| workers::listen(shared, fact, listener, acceptor),
                    );
                }
                rebind::Bound::Datagram { fact, endpoint } => {
                    self.register_listener(&fact)?;
                    self.listeners.push(fact.clone());
                    self.spawn_owner(
                        resources::Key::Listener(fact.key.clone()),
                        Some(fact.socket_id),
                        |shared| datagram::listen(shared, fact, endpoint),
                    );
                }
                rebind::Bound::Traversal {
                    fact,
                    reference,
                    socket,
                } => {
                    self.listeners.push(fact.clone());
                    let (commands, received) = mpsc::channel(8);
                    self.shared
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
                                buffers: fact.buffers,
                            },
                        );
                    endpoints.insert(
                        fact.socket_id,
                        traversal::Endpoint {
                            commands,
                            authorization: socket.authorization.clone(),
                        },
                    );
                    self.spawn_owner(
                        resources::Key::Listener(fact.key.clone()),
                        Some(fact.socket_id),
                        |shared| {
                            datagram::listen_traversal(
                                shared,
                                fact.socket_id,
                                Some(fact),
                                socket,
                                received,
                            )
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// New coordination sockets reach discovery and the traversal owner.
    fn publish_endpoints(
        &mut self,
        endpoints: BTreeMap<[u8; 32], traversal::Endpoint>,
    ) -> Result<(), Error> {
        let added_endpoints = !endpoints.is_empty();
        self.shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .extend(endpoints);
        if added_endpoints {
            self.shared.resource_changes.send_replace(());
        }
        if !self
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .is_empty()
            && !self.owners.contains_key(&resources::Key::Traversal)
            && let Some((gateway, _)) = self.traversal.clone()
        {
            self.spawn_owner(resources::Key::Traversal, None, |shared| {
                traversal::run(shared, gateway)
            });
        }
        Ok(())
    }

    /// Size limits of the selected Native UDP channels whose route MTU is
    /// known ([`Snapshot::mtu`]), without the rest of the snapshot: the
    /// packet owner reads them every turn, as it reads the carriers.
    pub fn path_mtu(&self) -> Result<BTreeMap<NodeId, LinkMtu>, Error> {
        self.check_view()?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        self.link_mtu(&links)
    }

    fn link_mtu(&self, links: &LinkTable) -> Result<BTreeMap<NodeId, LinkMtu>, Error> {
        let path_mtu = self.shared.path_mtu.lock().map_err(|_| Error::Worker)?;
        Ok(links
            .values()
            .filter(|link| link.scope.check().is_ok())
            .filter_map(|link| {
                let (_, path) = path_mtu.get(&link.fact.id)?;
                Some((
                    link.fact.peer,
                    LinkMtu::new(*path, link.fact.remote.is_ipv4())?,
                ))
            })
            .collect())
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.snapshot");
        let _profile = PROFILE.enter();
        self.check_view()?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        let linked: std::collections::BTreeSet<NodeId> = links
            .values()
            .filter(|link| link.scope.check().is_ok())
            .map(|link| link.fact.peer)
            .collect();
        let plan = self.shared.plan();
        let adjacencies = plan
            .peers()
            .filter(|peer| plan.adjacent(*peer) && !linked.contains(peer))
            .map(|peer| {
                (
                    peer,
                    if self.shared.wanted(peer) {
                        meshway_model::state::AdjacencyStatus::Unreachable
                    } else {
                        meshway_model::state::AdjacencyStatus::Idle
                    },
                )
            })
            .collect();
        let mtu = self.link_mtu(&links)?;
        Ok(Snapshot {
            adjacencies,
            mtu,
            unavailable: self.unavailable_listeners(),
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

    /// A State socket reference names the netmap and the live instance; with
    /// no instance yet, the socket is registered once one is bound.
    fn register_listener(&self, fact: &ListenerFact) -> Result<(), Error> {
        let Some(reference) = traversal::socket_reference(
            &self.shared,
            fact.socket_id,
            meshway_model::state::ResourceRef::Listener {
                key: fact.key.clone(),
            },
        )?
        else {
            return Ok(());
        };
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
                    buffers: fact.buffers,
                },
            );
        Ok(())
    }

    /// The signed boot identity of each selected peer channel; the packet
    /// owner retires circuits whose adjacent peer's boot changed.
    pub fn peer_boots(&self) -> Result<BTreeMap<NodeId, crate::Boot>, Error> {
        self.check_view()?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        Ok(links
            .values()
            .filter(|link| link.scope.check().is_ok())
            .map(|link| (link.fact.peer, link.fact.peer_boot))
            .collect())
    }

    /// This Manager's own boot identity, signed into every adjacency it
    /// makes. It outlives the Manager's internal restarts; a new Manager has
    /// a new one, which every adjacent peer sees as a restart of this Node.
    pub fn boot(&self) -> crate::Boot {
        self.shared.boot
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

    /// Whether this Manager's owners still run (it has not stopped).
    pub fn is_running(&self) -> bool {
        !self.shared.stop.is_cancelled()
    }

    /// The content this Manager currently applies.
    pub fn reference(&self) -> meshway_model::NetmapRef {
        self.shared.plan().reference
    }

    fn check_view(&self) -> Result<(), Error> {
        if self.shared.stop.is_cancelled() {
            return Err(Error::Unavailable);
        }
        Ok(())
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
            link.activity.record(&shared.interests);
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
        self.receive_events(1, true).await.pop().expect("one event")
    }

    /// Drain at most 32 already queued events per wakeup. Each frame receives
    /// the same current-channel and execution checks as single-event receive.
    /// The returned facts are observations, not transferable I/O authority.
    pub async fn recv_batch(&mut self) -> Vec<Event> {
        self.receive_events(32, true).await
    }

    /// Pause business ingress under host TX pressure, while bounded handshake,
    /// diagnostic and lifecycle events retain their independent scheduling lane.
    pub async fn recv_batch_with_data(&mut self, data: bool) -> Vec<Event> {
        self.receive_events(32, data).await
    }

    async fn receive_events(&mut self, limit: usize, data: bool) -> Vec<Event> {
        loop {
            let mut events = Vec::new();
            let mut controls = Vec::new();
            let count = tokio::select! { biased;
                _ = self.shared.stop.cancelled() => return vec![Event::Closed],
                Some(rebound) = self.rebound.recv() => {
                    match self.adopt(rebound).await {
                        Ok(true) => return vec![Event::Changed],
                        Ok(false) => continue,
                        Err(error) => {
                            tracing::warn!(%error, "Installing a rebound listener failed");
                            self.shared.stop.cancel();
                            return vec![Event::Closed];
                        }
                    }
                },
                count = self.control_events.recv_many(&mut controls, limit) => { events = controls; count },
                count = self.events.recv_many(&mut events, limit), if data => count,
            };
            if count == 0 || self.shared.stop.is_cancelled() {
                return vec![Event::Closed];
            }
            {
                let Ok(links) = self.shared.links.lock() else {
                    return vec![Event::Closed];
                };
                let mut checked = BTreeMap::new();
                events.retain(|event| match event {
                    Event::Frame { link, .. } => *checked.entry(link.id).or_insert_with(|| {
                        links.get(&link.peer).is_some_and(|current| {
                            let selected =
                                current.fact.id == link.id && current.scope.check().is_ok();
                            if selected {
                                current.activity.record(&self.shared.interests);
                            }
                            selected
                        })
                    }),
                    _ => true,
                });
            }
            if self.shared.stop.is_cancelled() {
                return vec![Event::Closed];
            }
            if !events.is_empty() {
                if meshway_logging::profile::enabled() {
                    meshway_logging::profile::event(
                        "manager.receive.batch",
                        match events.len() {
                            1 => "1",
                            2..=4 => "2-4",
                            5..=8 => "5-8",
                            9..=16 => "9-16",
                            _ => "17-32",
                        },
                    );
                    meshway_logging::profile::event(
                        "manager.event_queue.depth",
                        match self.events.len() {
                            0 => "0",
                            1..=8 => "1-8",
                            9..=32 => "9-32",
                            33..=64 => "33-64",
                            65..=256 => "65-256",
                            _ => "257+",
                        },
                    );
                    if let Some(Event::Frame { received_at, .. }) = events.first() {
                        static WALL: meshway_logging::profile::Metric =
                            meshway_logging::profile::Metric::new(
                                "manager.receive_to_consume.wall",
                            );
                        WALL.elapsed(received_at.elapsed());
                    }
                }
                if events.iter().any(|event| matches!(event, Event::Changed)) {
                    self.observe_remotes();
                }
                return events;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Bind State socket references to the instance that now qualifies this
    /// Node (a new instance after a Server outage, or the first one after an
    /// offline restart). Content and channels are unchanged; a Native UDP
    /// listener bound before any instance existed is rebound as a
    /// coordination socket, and dial-only traversal sockets are bound now,
    /// since coordination needs their State reference.
    pub async fn set_instance(
        &mut self,
        instance: Option<meshway_protocol::InstanceRequest>,
    ) -> Result<(), Error> {
        let first = {
            let mut current = self.shared.instance.lock().map_err(|_| Error::Worker)?;
            if *current == instance {
                return Ok(());
            }
            let first = current.is_none();
            current.clone_from(&instance);
            first
        };
        let Some(instance) = instance else {
            return Ok(());
        };
        for socket in self
            .shared
            .sockets
            .lock()
            .map_err(|_| Error::Worker)?
            .values_mut()
        {
            socket.reference.instance = instance.clone();
        }
        for fact in &self.listeners {
            self.register_listener(fact)?;
        }
        let links: Vec<_> = self
            .shared
            .links
            .lock()
            .map_err(|_| Error::Worker)?
            .values()
            .map(|link| link.fact.clone())
            .collect();
        for fact in links {
            self.shared.register_link_socket(&fact)?;
        }
        // Underlay authorizes attempt targets against this exact reference:
        // without the rebind every later offer (prepared under the new
        // instance) would be refused on activation and never punch.
        self.rebind_sockets()?;
        if !first || self.traversal.is_none() {
            return Ok(());
        }
        let coordinated = self
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let plain: Vec<_> = self
            .listeners
            .iter()
            .filter(|fact| {
                fact.protocol == TransportProtocol::NativeUdp
                    && !coordinated.contains(&fact.socket_id)
            })
            .map(|fact| resources::Key::Listener(fact.key.clone()))
            .collect();
        let retired = self.retire_keys(&plain);
        self.join_retired(retired).await?;
        // Also binds the dial-only traversal sockets skipped without an
        // instance; installed resources are kept as they are.
        self.install_resources().await
    }

    pub async fn shutdown(&mut self) -> Result<(), Error> {
        if diagnostic_event() {
            tracing::debug!(
                operation = "shutdown",
                netmap_serial = self.shared.plan().reference.serial.get(),
                listeners = self.listeners.len(),
                "Connectivity manager transition"
            );
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
        // Unadopted binds close now, not when this value is dropped: a
        // restart binds the same ports before the old Manager is replaced.
        self.rebound.close();
        while self.rebound.try_recv().is_ok() {}
        self.shared.links.lock().map_err(|_| Error::Worker)?.clear();
        self.listeners.clear();
        self.shared.unbound.with(BTreeMap::clear)?;
        self.shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .clear();
        self.events.close();
        self.control_events.close();
        while self.events.try_recv().is_ok() {}
        while self.control_events.try_recv().is_ok() {}
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

fn validate_scale(plan: &Plan, limits: Limits) -> Result<(), Error> {
    // A netmap lists every visible Node; only dial candidates and listeners
    // own resources here, and peers are acquired on demand.
    if plan.listeners.len() > limits.listeners
        || plan.dialers.len() > limits.candidates
        || plan
            .dialers
            .values()
            .try_fold(0usize, |count, dial| {
                count.checked_add(dial.endpoints.len())
            })
            .is_none_or(|count| count > limits.candidates)
    {
        return Err(Error::Capacity);
    }
    if plan
        .dialers
        .values()
        .any(|dial| dial.listener_key.is_empty() || dial.listener_key.len() > 128)
    {
        return Err(Error::Configuration);
    }
    Ok(())
}

/// The only link selection/installation point, shared by both acquisition
/// drivers. Authentication has completed before constructing this value.
struct ReadyPeer {
    fact: LinkFact,
    sender: Arc<SharedSender>,
    scope: Scope,
    /// The content whose keys authenticated this channel.
    plan: Arc<Plan>,
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
    // An acquisition authenticated under earlier content commits only if the
    // current content still admits its carrier with the same peer keys. Hold
    // the content read through installation: Manager's apply and a commit
    // therefore have one definite order.
    let current = shared.plans.borrow();
    if !Arc::ptr_eq(&ready.plan, &current)
        && (!ready.plan.peer_unchanged(&current, ready.fact.peer)
            || current.admit(ready.fact.peer, &ready.fact.carrier).is_err())
    {
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
            // The exact channel: a Native UDP one being reattached, or a
            // stream the edge orders after this Native UDP carrier.
            Replacement::Channel(id) => links
                .get(&ready.fact.peer)
                .is_some_and(|old| old.fact.id == id),
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
    // A blocked lock/diagnostic must not let a cancelled acquisition commit.
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
    let activity = Arc::new(interest::Activity::default());
    activity.record(&shared.interests);
    links.insert(
        ready.fact.peer,
        Link {
            fact: ready.fact,
            sender: ready.sender,
            scope: ready.scope,
            batch_scope: meshway_transport::ChannelScope::new(),
            activity,
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
    retire_for(shared, fact, "owner_retire")
}

fn retire_for(shared: &Shared, fact: &LinkFact, cause: &'static str) -> Result<(), Error> {
    let mut links = shared.links.lock().map_err(|_| Error::Worker)?;
    if links
        .get(&fact.peer)
        .is_some_and(|link| link.fact.id == fact.id)
        && let Some(link) = links.remove(&fact.peer)
    {
        diagnostic_link(shared, &link.fact, &link.scope, "retire", cause, None);
        link.scope.cancellation.cancel();
    }
    Ok(())
}

async fn connected(
    shared: Arc<Shared>,
    stream: Box<dyn meshway_transport::raw::Stream>,
    fact: LinkFact,
    plan: Arc<Plan>,
    opening: Arc<scope::Opening>,
    was_selected: &mut bool,
) -> Result<(), Error> {
    let scope = shared.scope(Some((fact.peer, fact.carrier.clone())), false);
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
    let sender = Arc::new(SharedSender::new(sender));
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
            plan,
            supersedes: Replacement::Ranked,
            attempt: None,
        },
    )?;
    if !selected {
        close_sender(&sender).await?;
        return Ok(());
    }
    if let Err(error) = opening.release() {
        scope.cancellation.cancel();
        retire(&shared, &fact)?;
        let _ = close_sender(&sender).await;
        return Err(error);
    }
    *was_selected = true;
    // Runs until this channel is retired: by Manager's apply when the content
    // no longer admits it, by replacement, idle retirement or stream EOF.
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
    let result = scope.run(receive).await;
    scope.cancellation.cancel();
    drop(receiver);
    retire(&shared, &fact)?;
    close_sender(&sender).await?;
    let _ = shared
        .event(&shared.scope(None, false), Event::Changed)
        .await;
    result
}

/// Stop a shared sender and join its owner. The lock is held for the
/// synchronous close only, never across the join (see [`SharedSender`]).
async fn close_sender(sender: &SharedSender) -> Result<(), Error> {
    let closing = sender
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .close();
    closing.await.map_err(|_| Error::Worker)
}

/// What survives a Manager restart for new own identity or resources.
#[derive(Default)]
struct Carried {
    interests: Arc<interest::Interests>,
    facts: Arc<facts::Store>,
    instance: Option<meshway_protocol::InstanceRequest>,
    /// Circuits outlive an internal restart; peers must not see a new boot.
    boot: Option<crate::Boot>,
}
