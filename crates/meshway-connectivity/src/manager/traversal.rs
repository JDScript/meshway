//! Bounded acquisition orchestration. Endpoint commands reserve actual receiver
//! slots; HTTP work runs independently of the host packet/sample worker.
use super::*;
use crate::traversal::{Operation, Reply, TraversalGateway};
use meshway_control::connectivity::{
    AttemptScope, AttemptView, DiscoveryContext, Outcome, ValidatedOffer, prepare_request,
};
use meshway_model::{
    UdpTraversalId,
    state::{ResourceRef, SocketNetwork},
};
use meshway_protocol::{
    connectivity::SocketReference,
    state::{Freshness, VisibleReport},
};
use tokio::{
    sync::oneshot,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Endpoint {
    pub commands: mpsc::Sender<Command>,
    pub authorization: Arc<dyn crate::DatagramAuthorization>,
}
pub(super) enum Command {
    Discover {
        context: DiscoveryContext,
        server: String,
        targets: Vec<SocketAddr>,
        /// A local diagnostic asked for this round: also probe the path
        /// size toward each target.
        size_probe: bool,
    },
    Reserve {
        offer: ValidatedOffer,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Activate(AttemptScope),
    Cancel(UdpTraversalId),
}

/// The State reference of a socket on the current content, or `None` while
/// no live instance qualifies the Node.
pub(super) fn socket_reference(
    shared: &Shared,
    id: [u8; 32],
    resource: ResourceRef,
) -> Result<Option<SocketReference>, Error> {
    let Some(instance) = shared.instance.lock().map_err(|_| Error::Worker)?.clone() else {
        return Ok(None);
    };
    let plan = shared.plan();
    Ok(Some(SocketReference {
        node_id: plan.node,
        instance,
        netmap: plan.reference,
        socket_id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
        socket_generation: shared.generation,
        resource,
    }))
}

/// Native UDP carriers with `peer` the plan admits, in either direction.
fn udp_carrier(plan: &Plan, carrier: &Carrier) -> bool {
    let peer = if carrier.dialer == plan.node {
        carrier.listener
    } else {
        carrier.dialer
    };
    carrier.protocol == TransportProtocol::NativeUdp && plan.admit(peer, carrier).is_ok()
}

struct Pending {
    offer: ValidatedOffer,
    endpoint: [u8; 32],
    active: bool,
    accept_at: Instant,
}

// Capability knowledge from the latest bounded metadata resolution, not I/O
// authority. A socket whose address family no STUN server serves cannot poll
// forever; failures remain retryable and metadata expiry forces a fresh query.
struct Observers {
    expires: Instant,
    unavailable: std::collections::BTreeSet<[u8; 32]>,
}

// Kept across content and resource changes: churn cannot reset admission.
// Acquisition is bounded to 30 starts in any half-open minute, paced at 250 ms.
const QUERY_INTERVAL: Duration = Duration::from_millis(250);
/// Every traversal socket observes its mapping on its own cadence, whether or
/// not any peer is interested: ~30 s idle, ~10 s while it carries a Native
/// UDP channel or an attempt, each with a fixed per-socket jitter of ±10 %.
const STUN_IDLE: Duration = Duration::from_secs(30);
const STUN_ACTIVE: Duration = Duration::from_secs(10);
/// A round without a validated response retries after 2 s, doubling up to
/// the idle cadence; a metadata query that yields no STUN context backs off
/// the same way. Neither consumes the acquisition budget.
const STUN_RETRY: Duration = Duration::from_secs(2);

struct QuerySchedule {
    next: Instant,
    observation_at: Instant,
    observation_last: Instant,
    observation_backoff: Duration,
    acquisitions: std::collections::VecDeque<Instant>,
}
impl QuerySchedule {
    fn new(now: Instant) -> Self {
        Self {
            next: now,
            observation_at: now,
            observation_last: now,
            observation_backoff: STUN_RETRY,
            acquisitions: std::collections::VecDeque::with_capacity(30),
        }
    }

    fn admit(&mut self, now: Instant, acquisition: bool, observation: bool) -> bool {
        if now < self.next {
            return false;
        }
        while self
            .acquisitions
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.acquisitions.pop_front();
        }
        if observation && now >= self.observation_at {
            self.observation_last = now;
            self.observation_at = now + self.observation_backoff;
        } else if acquisition && self.acquisitions.len() < 30 {
            self.acquisitions.push_back(now);
        } else {
            return false;
        }
        self.next = now + QUERY_INTERVAL;
        true
    }

    /// Whether the last admitted observation query produced a usable STUN
    /// context; its outcome paces the next one.
    fn observed(&mut self, usable: bool) {
        self.observation_backoff = if usable {
            STUN_RETRY
        } else {
            (self.observation_backoff * 2).min(STUN_IDLE)
        };
        self.observation_at = self.observation_last + self.observation_backoff;
    }
}

/// One socket's observation rounds: when the last one was sent and how many
/// in a row ended without a validated response.
#[derive(Clone, Copy)]
struct Round {
    sent: Instant,
    failures: u32,
}

/// The socket's fixed cadence: idle or active base with its own jitter.
fn cadence(id: &[u8; 32], active: bool) -> Duration {
    let base = if active { STUN_ACTIVE } else { STUN_IDLE };
    base.mul_f64(0.9 + 0.2 * f64::from(id[0]) / 255.0)
}

/// Due when the cadence since the last validated response elapsed (or none
/// arrived yet), and not before the retry delay of an unanswered round.
fn observation_due(
    confirmed: Option<Instant>,
    round: Option<Round>,
    cadence: Duration,
    now: Instant,
) -> bool {
    if let Some(round) = round
        && confirmed.is_none_or(|at| at < round.sent)
        && now < round.sent + retry(round.failures)
    {
        return false;
    }
    confirmed.is_none_or(|at| now >= at + cadence)
}

fn retry(failures: u32) -> Duration {
    STUN_RETRY
        .saturating_mul(1 << failures.min(4))
        .min(STUN_IDLE)
}

fn needs_acquisition(shared: &Shared, pending: &BTreeMap<UdpTraversalId, Pending>) -> bool {
    let plan = &*shared.plan();
    let Ok(links) = shared.links.lock() else {
        return false;
    };
    plan.peers().any(|peer| {
        // The selected owner retires unhealthy channels. A selected channel is
        // not a vacant physical slot: prepare() must wait for that owner too.
        if links.contains_key(&peer) || !shared.wanted(peer) {
            return false;
        }
        // The exact admitted offer already supplies this peer's acquisition
        // context. Accept rechecks its authority; another metadata query must
        // not sit ahead of the final bilateral acceptance in a short window.
        if pending
            .values()
            .any(|value| value.offer.peer().candidate.socket.node_id == peer)
        {
            return false;
        }
        udp_edge(plan, peer)
    })
}

/// Whether `plan` admits a Native UDP carrier with `peer` in either
/// direction: this Node may initiate coordination for it or be its target.
fn udp_edge(plan: &Plan, peer: NodeId) -> bool {
    plan.dialers.values().any(|dial| {
        dial.peer == peer
            && udp_carrier(
                plan,
                &Carrier {
                    dialer: plan.node,
                    listener: peer,
                    listener_key: dial.listener_key.clone(),
                    protocol: dial.parameters.transport(),
                },
            )
    }) || plan.listeners.iter().any(|(key, listener)| {
        let (protocol, enabled) = provider::declaration(listener);
        enabled
            && udp_carrier(
                plan,
                &Carrier {
                    dialer: peer,
                    listener: plan.node,
                    listener_key: key.clone(),
                    protocol,
                },
            )
    })
}

/// The socket carries a selected Native UDP channel or an attempt.
fn socket_active(
    shared: &Shared,
    pending: &BTreeMap<UdpTraversalId, Pending>,
    id: &[u8; 32],
) -> bool {
    pending.values().any(|value| value.endpoint == *id)
        || shared.links.lock().is_ok_and(|links| {
            links.values().any(|link| {
                link.fact.socket_id == *id
                    && link.fact.carrier.protocol == TransportProtocol::NativeUdp
                    && link.scope.check().is_ok()
            })
        })
}

fn wall_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Scheduling state owned by the traversal owner for its whole life. Only
/// these budgets and cursors survive new content; offers, discovery contexts
/// and observer metadata belong to one content and keep their own deadlines.
struct Schedule {
    queries: QuerySchedule,
    /// Observation rounds of each current traversal socket.
    rounds: BTreeMap<[u8; 32], Round>,
    /// Fair eight-peer batch cursor for prepare.
    after_peer: Option<NodeId>,
}
impl Schedule {
    fn new() -> Self {
        Self {
            queries: QuerySchedule::new(Instant::now()),
            rounds: BTreeMap::new(),
            after_peer: None,
        }
    }
    /// A new socket has no round and no confirmation: it is due at once.
    fn sockets(&mut self, endpoints: &BTreeMap<[u8; 32], Endpoint>) {
        self.rounds.retain(|id, _| endpoints.contains_key(id));
    }
}

pub(super) async fn run(shared: Arc<Shared>, gateway: Arc<dyn TraversalGateway>) {
    let mut plans = shared.plans.subscribe();
    let mut resources = shared.resource_changes.subscribe();
    let mut schedule = Schedule::new();
    while !shared.stop.is_cancelled() {
        let endpoints = match shared.traversal_endpoints.lock() {
            Ok(endpoints) => endpoints.clone(),
            Err(_) => return,
        };
        schedule.sockets(&endpoints);
        plans.borrow_and_update();
        resources.borrow_and_update();
        run_content(
            shared.clone(),
            &gateway,
            &endpoints,
            &mut plans,
            &mut resources,
            &mut schedule,
        )
        .await;
    }
}

/// One content's discovery, offers and attempts, until new content or new
/// traversal sockets restart them.
async fn run_content(
    shared: Arc<Shared>,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    plans: &mut tokio::sync::watch::Receiver<Arc<Plan>>,
    resources: &mut tokio::sync::watch::Receiver<()>,
    schedule: &mut Schedule,
) {
    let Schedule {
        queries,
        rounds,
        after_peer,
    } = schedule;
    let scope = shared.scope(None, false);
    let mut pending: BTreeMap<UdpTraversalId, Pending> = BTreeMap::new();
    let mut retired: BTreeMap<UdpTraversalId, u64> = BTreeMap::new();
    let mut prepare_at = Instant::now();
    let mut context: Option<DiscoveryContext> = None;
    let mut observers: Option<Observers> = None;
    let reference = shared.plan().reference;
    let work = async {
        loop {
            let now = wall_ms();
            retired.retain(|_, expires| *expires > now);
            pending.retain(|id, value| {
                if value.offer.ensure_current().is_ok() {
                    return true;
                }
                value.offer.cancel();
                if let Some(endpoint) = endpoints.get(&value.endpoint) {
                    let _ = endpoint.commands.try_send(Command::Cancel(*id));
                }
                false
            });
            // Finish issued control work before unrelated discovery HTTP.
            for value in pending.values_mut() {
                if !shared.links.lock().is_ok_and(|links| {
                    links.contains_key(&value.offer.peer().candidate.socket.node_id)
                }) {
                    accept(gateway, endpoints, value).await;
                }
            }
            // A successful independently authenticated channel also supersedes
            // waiting offers for that peer (for example another address family).
            // Their original slots expire normally; they cannot keep polling a
            // healthy selected link merely because their phase is still Waiting.
            // A selected channel suppresses acquisition, never observation.
            let acquisition = !endpoints.is_empty() && needs_acquisition(&shared, &pending);
            let confirmed = shared
                .stun_confirmed
                .lock()
                .map(|confirmed| confirmed.clone())
                .unwrap_or_default();
            let requested = shared
                .observation_request
                .lock()
                .map(|requested| *requested)
                .unwrap_or_default();
            let due: Vec<[u8; 32]> = endpoints
                .keys()
                .filter(|id| {
                    observers.as_ref().is_none_or(|known| {
                        Instant::now() >= known.expires || !known.unavailable.contains(*id)
                    }) && (observation_due(
                        confirmed.get(*id).copied(),
                        rounds.get(*id).copied(),
                        cadence(id, socket_active(&shared, &pending, id)),
                        Instant::now(),
                    ) || requested
                        .is_some_and(|at| rounds.get(*id).is_none_or(|round| round.sent < at)))
                })
                .copied()
                .collect();
            // A live discovery context already authorizes same-socket STUN;
            // metadata is queried only when none is usable. New content or
            // sockets drop this context; query admission keeps its budget.
            // Rounds a local diagnostic asked for, before they are recorded.
            let probed: Vec<[u8; 32]> = due
                .iter()
                .filter(|id| {
                    requested.is_some_and(|at| rounds.get(*id).is_none_or(|round| round.sent < at))
                })
                .copied()
                .collect();
            let observation_query = !due.is_empty()
                && context
                    .as_ref()
                    .is_none_or(|context| context.ensure_current().is_err());
            if queries.admit(Instant::now(), acquisition, observation_query) {
                let query = gateway.request(Operation::Query(reference)).await;
                if observation_query {
                    queries.observed(matches!(query, Ok(Reply::Query(Outcome::Current(_)))));
                }
                if let Ok(Reply::Query(Outcome::Current(discovery))) = query {
                    context = Some(discovery.context);
                    observers = None;
                    let visible: Vec<_> = discovery
                        .attempts
                        .iter()
                        .map(|attempt| match attempt {
                            AttemptView::Waiting(offer) => offer.offer().attempt_id,
                            AttemptView::Active(scope) => {
                                scope.validated_offer().offer().attempt_id
                            }
                        })
                        .collect();
                    pending.retain(|id, value| {
                        let keep = visible.contains(id) && value.offer.ensure_current().is_ok();
                        if !keep {
                            retired.insert(*id, value.offer.offer().expires_at_ms);
                            value.offer.cancel();
                            if let Some(endpoint) = endpoints.get(&value.endpoint) {
                                let _ = endpoint.commands.try_send(Command::Cancel(*id));
                            }
                        }
                        keep
                    });
                    // Owner slots retain their ORIGINAL offer. A later read of the
                    // same offer never replaces it or extends its deadline.
                    for attempt in discovery.attempts {
                        let offer = match attempt {
                            AttemptView::Waiting(offer) => offer,
                            AttemptView::Active(scope) => scope.validated_offer().clone(),
                        };
                        let id = offer.offer().attempt_id;
                        if pending.contains_key(&id)
                            || retired.contains_key(&id)
                            || pending.len() >= 8
                            || retired.len() >= 128
                        {
                            continue;
                        }
                        if gateway.published(&offer.local().candidate)
                            && let Some(endpoint) = matching_endpoint(&shared, &offer)
                            && reserve(endpoints, endpoint, &offer).await.is_ok()
                        {
                            let value = pending.entry(id).or_insert(Pending {
                                offer,
                                endpoint,
                                active: false,
                                accept_at: Instant::now(),
                            });
                            accept(gateway, endpoints, value).await;
                        }
                    }
                } else if matches!(query, Ok(Reply::Query(Outcome::Unavailable(_)))) {
                    context = None;
                    for (id, value) in std::mem::take(&mut pending) {
                        value.offer.cancel();
                        retired.insert(id, value.offer.offer().expires_at_ms);
                        if let Some(endpoint) = endpoints.get(&value.endpoint) {
                            let _ = endpoint.commands.try_send(Command::Cancel(id));
                        }
                    }
                }
            }
            // Query admission is not the clock for an already issued offer or
            // discovery authority. Advance the original bounded operations
            // before their own deadlines without issuing another metadata
            // query or extending any deadline.
            if let Some(context) = context
                .as_ref()
                .filter(|context| context.ensure_current().is_ok())
            {
                if !due.is_empty() {
                    for id in &due {
                        let failures = rounds.get(id).map_or(0, |round| {
                            if confirmed.get(id).is_none_or(|at| *at < round.sent) {
                                round.failures.saturating_add(1)
                            } else {
                                0
                            }
                        });
                        rounds.insert(
                            *id,
                            Round {
                                sent: Instant::now(),
                                failures,
                            },
                        );
                    }
                    let mut unavailable = observers
                        .take()
                        .filter(|known| Instant::now() < known.expires)
                        .map(|known| known.unavailable)
                        .unwrap_or_default();
                    for id in &due {
                        let Some(endpoint) = endpoints.get(id) else {
                            continue;
                        };
                        let family = shared.sockets.lock().ok().and_then(|sockets| {
                            sockets.get(id).map(|socket| socket.bound_address.is_ipv4())
                        });
                        let Some(ipv4) = family else {
                            continue;
                        };
                        let mut served = false;
                        for server in context.servers() {
                            let resolved = tokio::select! { biased;
                                _ = context.ended() => break,
                                result = endpoint.authorization.authorize_stun(context, &server.id) => result,
                            };
                            match resolved {
                                Ok(targets) => {
                                    let targets: Vec<_> = targets
                                        .into_iter()
                                        .filter(|target| target.is_ipv4() == ipv4)
                                        .collect();
                                    if !targets.is_empty() {
                                        served = true;
                                        let _ = endpoint.commands.try_send(Command::Discover {
                                            context: context.clone(),
                                            server: server.id.clone(),
                                            targets,
                                            size_probe: probed.contains(id),
                                        });
                                    }
                                }
                                // A failed resolution is retryable, not a
                                // missing address family.
                                Err(_) => served = true,
                            }
                        }
                        if served {
                            unavailable.remove(id);
                        } else {
                            unavailable.insert(*id);
                        }
                    }
                    observers = Some(Observers {
                        expires: context.expires_at().into(),
                        unavailable,
                    });
                }
                if Instant::now() >= prepare_at && pending.len() < 8 {
                    prepare_at = Instant::now() + Duration::from_secs(1);
                    let _ = prepare(
                        &shared,
                        gateway,
                        endpoints,
                        context,
                        &mut pending,
                        after_peer,
                    )
                    .await;
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        #[allow(unreachable_code)]
        Ok(())
    };
    tokio::select! { biased;
        _ = plans.changed() => {},
        _ = resources.changed() => {},
        _ = scope.run(work) => {},
    }
    for (id, value) in pending {
        value.offer.cancel();
        if let Some(endpoint) = endpoints.get(&value.endpoint) {
            let _ = endpoint.commands.try_send(Command::Cancel(id));
        }
    }
}

pub(super) fn matching_endpoint(shared: &Shared, offer: &ValidatedOffer) -> Option<[u8; 32]> {
    let sockets = shared.sockets.lock().ok()?;
    sockets.iter().find_map(|(id, fact)| {
        (fact.reference == offer.local().candidate.socket
            && socket_compatible(&shared.plan(), &offer.offer().carrier, fact)
            && fact.endpoints.iter().any(|observation| {
                fact.candidate(observation)
                    .is_ok_and(|candidate| candidate == offer.local().candidate)
                    && observation.address == offer.local().address
            }))
        .then_some(*id)
    })
}

fn socket_compatible(plan: &Plan, carrier: &Carrier, fact: &SocketFact) -> bool {
    if plan.node == carrier.listener {
        return fact.reference.resource
            == (ResourceRef::Listener {
                key: carrier.listener_key.clone(),
            });
    }
    let key = format!("{}/{}", carrier.listener, carrier.listener_key);
    let Some(dial) = plan.dialers.get(&key) else {
        return false;
    };
    let meshway_transport::spec::DialParameters::NativeUdp { options } = &dial.parameters else {
        return false;
    };
    match &fact.reference.resource {
        ResourceRef::Dial { key: bound } => {
            *bound == key && dial_bind_matches(options, fact.bound_address)
        }
        ResourceRef::Listener { key } => plan
            .listeners
            .get(key)
            .is_some_and(|listener| listener_covers(listener, fact.bound_address, options)),
        ResourceRef::Coordination { .. } => false,
    }
}

pub(super) fn listener_covers(
    listener: &meshway_transport::spec::PreparedListener,
    local: SocketAddr,
    options: &meshway_transport::spec::NativeUdpDialer,
) -> bool {
    dial_bind_matches(options, local)
        && matches!(listener.declaration(), meshway_transport::spec::Listener::NativeUdp(listener)
            if listener.enabled && listener.bind_interface.is_empty() && listener.socket == options.socket)
}

fn dial_bind_matches(
    options: &meshway_transport::spec::NativeUdpDialer,
    local: SocketAddr,
) -> bool {
    options.enabled
        && options.bind_interface.is_empty()
        && (options.local_address.is_empty()
            || options.local_address.parse::<std::net::IpAddr>().ok() == Some(local.ip()))
}

fn peer_batch(carriers: &BTreeMap<NodeId, Carrier>, after: &mut Option<NodeId>) -> Vec<NodeId> {
    let peers: Vec<_> = carriers
        .keys()
        .filter(|peer| after.is_none_or(|after| **peer > after))
        .chain(
            carriers
                .keys()
                .filter(|peer| after.is_some_and(|after| **peer <= after)),
        )
        .take(8)
        .copied()
        .collect();
    if let Some(last) = peers.last() {
        *after = Some(*last);
    }
    peers
}

async fn reserve(
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    id: [u8; 32],
    offer: &ValidatedOffer,
) -> Result<(), Error> {
    let (reply, receiver) = oneshot::channel();
    endpoints
        .get(&id)
        .ok_or(Error::Unavailable)?
        .commands
        .try_send(Command::Reserve {
            offer: offer.clone(),
            reply,
        })
        .map_err(|_| Error::Capacity)?;
    tokio::select! { biased;
        _ = offer.ended() => Err(Error::Authority),
        result = tokio::time::timeout(Duration::from_secs(1), receiver) => result.map_err(|_| Error::Cancelled)?.map_err(|_| Error::Unavailable)?,
    }
}

async fn accept(
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    pending: &mut Pending,
) {
    if pending.active
        || pending.offer.ensure_current().is_err()
        || Instant::now() < pending.accept_at
    {
        return;
    }
    pending.accept_at = Instant::now() + Duration::from_millis(500);
    match gateway
        .request(Operation::Accept(pending.offer.clone()))
        .await
    {
        Ok(Reply::Accept(Outcome::Current(AttemptView::Active(active))))
            if pending.offer.ensure_current().is_ok()
                && active.ensure_current().is_ok()
                && active.validated_offer().offer() == pending.offer.offer() =>
        {
            if let Some(endpoint) = endpoints.get(&pending.endpoint) {
                pending.active = endpoint
                    .commands
                    .try_send(Command::Activate(active))
                    .is_ok();
            }
        }
        Ok(Reply::Accept(Outcome::Unavailable(_))) => pending.offer.cancel(),
        // Waiting and uncertain RPC outcomes retain the original receiver slot
        // and authority. A later accept retries exactly this offer; neither a
        // new query guard nor prepare alone can authorize activation.
        _ => {}
    }
}

async fn prepare(
    shared: &Shared,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    context: &DiscoveryContext,
    pending: &mut BTreeMap<UdpTraversalId, Pending>,
    after_peer: &mut Option<NodeId>,
) -> Result<(), Error> {
    let plan = &*shared.plan();
    if context.netmap() != plan.reference {
        return Ok(());
    }
    let now = wall_ms();
    let mut carriers = BTreeMap::new();
    for peer in plan.dial_peers() {
        if !shared.wanted(peer) {
            continue;
        }
        // The edge's carrier order; the first admitted Native UDP one.
        for (_, dial) in plan.outgoing(peer)? {
            let carrier = Carrier {
                dialer: plan.node,
                listener: dial.peer,
                listener_key: dial.listener_key.clone(),
                protocol: dial.parameters.transport(),
            };
            if udp_carrier(plan, &carrier) {
                carriers.entry(dial.peer).or_insert(carrier);
                break;
            }
        }
    }
    // Either Node with a need prepares its own dialer carrier; the peer
    // participates on reading the offer, without a need of its own. When both
    // prepare at once the Server admits one live attempt per pair and address
    // family, and the other Node accepts that offer instead.
    carriers.retain(|peer, _| {
        !pending
            .values()
            .any(|pending| pending.offer.peer().candidate.socket.node_id == *peer)
            // Let the selected owner finish its retirement before a new acquisition.
            && !shared.links.lock().is_ok_and(|links| links.contains_key(peer))
    });
    // Advance before the RPC, including errors/no observations. An unavailable
    // prefix must not starve other peers; the cursor survives new content.
    let peers = peer_batch(&carriers, after_peer);
    if peers.is_empty() {
        return Ok(());
    }
    let Reply::Peers(view) = gateway
        .request(Operation::Peers {
            context: context.clone(),
            peers,
        })
        .await?
    else {
        return Err(Error::Configuration);
    };
    if view.state_epoch != context.state_epoch() {
        return Ok(());
    }
    let sockets: Vec<_> = shared
        .sockets
        .lock()
        .map_err(|_| Error::Worker)?
        .iter()
        .map(|(id, fact)| (*id, fact.clone()))
        .collect();
    for (peer, node) in view.nodes {
        if pending.len() >= 8 {
            break;
        }
        if node.freshness != Freshness::Current {
            tracing::debug!(%peer, freshness = ?node.freshness, cause = "peer_not_current", "Traversal preparation has no usable candidate");
            continue;
        }
        let Some(VisibleReport::Peer { report }) = node.report else {
            tracing::debug!(%peer, cause = "peer_report_unavailable", "Traversal preparation has no usable candidate");
            continue;
        };
        let Some(carrier) = carriers.get(&peer) else {
            continue;
        };
        let mut remote_candidates = 0usize;
        let mut compatible_sockets = 0usize;
        let mut local_candidates = 0usize;
        let selected = report
            .endpoints
            .iter()
            .filter(|remote| {
                remote.network == SocketNetwork::Udp
                    && remote.resource
                        == (ResourceRef::Listener {
                            key: carrier.listener_key.clone(),
                        })
                    && remote.candidate.expires_at.timestamp_millis()
                        > i64::try_from(now).unwrap_or(i64::MAX)
            })
            .find_map(|remote| {
                remote_candidates += 1;
                let address =
                    meshway_control::connectivity::udp_recipient(&remote.candidate.address).ok()?;
                sockets.iter().find_map(|(id, fact)| {
                    if fact.bound_address.is_ipv4() != address.is_ipv4()
                        || !socket_compatible(plan, carrier, fact)
                    {
                        return None;
                    }
                    compatible_sockets += 1;
                    fact.endpoints.iter().find_map(|observation| {
                        if observation.expires_at.timestamp_millis() <= i64::try_from(now).ok()? {
                            return None;
                        }
                        let local = fact.candidate(observation).ok()?;
                        local_candidates += 1;
                        gateway
                            .published(&local)
                            .then(|| (*id, local, remote.candidate_reference.clone()))
                    })
                })
            });
        let Some((endpoint, local, remote)) = selected else {
            tracing::debug!(%peer, remote_candidates, compatible_sockets, local_candidates,
                visible_endpoints = report.endpoints.len(), local_sockets = sockets.len(),
                netmap_serial = context.netmap().serial.get(),
                cause = "no_acknowledged_pair", "Traversal preparation has no usable candidate");
            continue;
        };
        let request = prepare_request(context, carrier.clone(), local, remote)
            .map_err(|_| Error::Authority)?;
        // Retry delivery uncertainty with precisely the same request/deadline.
        let operation = || Operation::Prepare {
            context: context.clone(),
            request: Box::new(request.clone()),
        };
        let response = match gateway.request(operation()).await {
            Ok(reply) => Ok(reply),
            Err(_) => gateway.request(operation()).await,
        };
        if let Ok(Reply::Prepare(Outcome::Current(offer))) = response {
            if reserve(endpoints, endpoint, &offer).await.is_ok() {
                // Install before the RPC so cancellation/uncertainty always
                // retains the exact reserved slot in this content's cleanup set.
                // Accept immediately, before another peer's prepare or query
                // consumes the short bilateral acquisition window.
                let value = pending.entry(offer.offer().attempt_id).or_insert(Pending {
                    offer,
                    endpoint,
                    active: false,
                    accept_at: Instant::now(),
                });
                accept(gateway, endpoints, value).await;
            } else {
                offer.cancel();
                let _ = gateway.request(Operation::Cancel(offer)).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{datagram::tests::shared, tests::netmap::support::Netmaps};

    #[tokio::test]
    async fn a_listener_only_coordination_target_participates_without_interest() {
        let mut fixture = Netmaps::start(false, false).await;
        let ids = fixture.ids();
        let quiet = |index: usize| {
            let (shared, _) = shared(&fixture.desired[index], &fixture.keys[index]);
            !needs_acquisition(&shared, &BTreeMap::new())
        };
        // Every edge is dialable both ways: nobody coordinates without a need.
        assert!(quiet(0) && quiet(1));
        let b = ids[1];
        fixture
            .publish(|cfg| {
                cfg.nodes.get_mut(&b).unwrap().dialers =
                    meshway_configuration::Input::Value(Default::default())
            })
            .await;
        // b can dial nobody: a's edge to b is resident, so a initiates and b,
        // the target, queries for offers. Both observe their mappings on
        // every socket regardless (see the cadence test).
        for index in [0, 1] {
            let (shared, _) = shared(&fixture.desired[index], &fixture.keys[index]);
            let peer = ids[1 - index];
            assert!(shared.wanted(peer));
            assert!(needs_acquisition(&shared, &BTreeMap::new()));
        }
        fixture.close().await;
    }

    #[test]
    fn unneeded_queries_are_suppressed_and_churn_cannot_reset_the_budget() {
        let start = Instant::now();
        let mut schedule = QuerySchedule::new(start);
        let mut queries = 0;
        for tick in 0..2400 {
            let now = start + Duration::from_millis(tick * 250);
            assert!(
                !schedule.admit(now, false, false),
                "neither acquisition nor observation due"
            );
            // Content/resource changes share this schedule, including a retry
            // immediately after a previous request failed or was cancelled.
            queries += usize::from(schedule.admit(now, true, false));
            assert!(!schedule.admit(now, true, false));
        }
        assert_eq!(queries, 300, "at most 30 requests/minute for ten minutes");
    }

    #[test]
    fn observation_queries_survive_saturated_acquisition_and_back_off_without_stun() {
        let start = Instant::now();
        let mut schedule = QuerySchedule::new(start);
        for tick in 0..30 {
            assert!(schedule.admit(start + Duration::from_millis(tick * 250), true, false));
        }
        let due = start + Duration::from_secs(8);
        assert!(!schedule.admit(due, true, false));
        assert!(schedule.admit(due, false, true));
        assert!(!schedule.admit(due + Duration::from_secs(1), true, true));
        // A Server without STUN: 4, 8, 16, then the idle cadence.
        let mut at = due;
        for wait in [4, 8, 16, 30, 30] {
            schedule.observed(false);
            assert!(!schedule.admit(at + Duration::from_secs(wait) - QUERY_INTERVAL, false, true));
            at += Duration::from_secs(wait);
            assert!(schedule.admit(at, false, true));
        }
        schedule.observed(true);
        assert!(schedule.admit(at + STUN_RETRY, false, true));
        assert!(schedule.admit(start + Duration::from_secs(600), true, false));
    }

    #[test]
    fn every_socket_observes_on_its_own_cadence_without_any_interest() {
        let start = Instant::now();
        let id = [128; 32];
        let idle = cadence(&id, false);
        let active = cadence(&id, true);
        assert!(idle > Duration::from_secs(27) && idle < Duration::from_secs(33));
        assert!(active > Duration::from_secs(9) && active < Duration::from_secs(11));
        assert_ne!(
            cadence(&[0; 32], false),
            cadence(&[255; 32], false),
            "jittered"
        );
        // A new socket is due at once; a confirmation defers it by its cadence.
        assert!(observation_due(None, None, idle, start));
        let sent = Round {
            sent: start,
            failures: 0,
        };
        let confirmed = start + Duration::from_millis(40);
        assert!(!observation_due(
            Some(confirmed),
            Some(sent),
            idle,
            confirmed + idle / 2
        ));
        assert!(observation_due(
            Some(confirmed),
            Some(sent),
            idle,
            confirmed + idle
        ));
        assert!(!observation_due(
            Some(confirmed),
            Some(sent),
            active,
            confirmed + active / 2
        ));
        assert!(observation_due(
            Some(confirmed),
            Some(sent),
            active,
            confirmed + active
        ));
        // An unanswered round retries after 2 s, doubling up to the idle cadence.
        for (failures, wait) in [(0, 2), (1, 4), (2, 8), (3, 16), (4, 30), (9, 30)] {
            let round = Round {
                sent: start,
                failures,
            };
            let wait = Duration::from_secs(wait);
            assert!(!observation_due(
                None,
                Some(round),
                idle,
                start + wait - QUERY_INTERVAL
            ));
            assert!(observation_due(None, Some(round), idle, start + wait));
            // An older confirmation does not answer a later round.
            assert!(!observation_due(
                Some(start - Duration::from_secs(60)),
                Some(round),
                idle,
                start + wait - QUERY_INTERVAL
            ));
        }
    }

    #[test]
    fn unavailable_peer_prefix_does_not_starve_later_batches_across_churn() {
        let node = |id| NodeId::new(uuid::Uuid::from_u128(id)).unwrap();
        let mut carriers: BTreeMap<_, _> = (1..=19)
            .map(|id| {
                let peer = node(id);
                (
                    peer,
                    Carrier {
                        dialer: node(100),
                        listener: peer,
                        listener_key: "udp".into(),
                        protocol: TransportProtocol::NativeUdp,
                    },
                )
            })
            .collect();
        let mut after = None;
        let mut queried = std::collections::BTreeSet::new();
        // No peers become pending/connected: model missing State or failed RPCs.
        for _ in 0..3 {
            let batch = peer_batch(&carriers, &mut after);
            assert_eq!(batch.len(), 8);
            assert_eq!(
                batch
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                8
            );
            queried.extend(batch);
        }
        assert_eq!(queried, carriers.keys().copied().collect());
        // A publication may remove the cursor peer. No reset to the unavailable
        // prefix and no requirement for the cursor's key to remain in the map.
        carriers.remove(&after.unwrap());
        carriers.retain(|peer, _| *peer > node(8));
        queried.clear();
        for _ in 0..2 {
            queried.extend(peer_batch(&carriers, &mut after));
        }
        assert_eq!(queried, carriers.keys().copied().collect());
        carriers.clear();
        assert!(peer_batch(&carriers, &mut after).is_empty());
    }
}
