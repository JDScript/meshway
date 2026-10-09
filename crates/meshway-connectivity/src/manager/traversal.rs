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
    },
    Reserve {
        offer: ValidatedOffer,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Activate(AttemptScope),
    Cancel(UdpTraversalId),
}

/// The State reference of a socket of this epoch, or `None` while no live
/// instance qualifies the Node.
pub(super) fn socket_reference(
    shared: &EpochContext,
    id: [u8; 32],
    resource: ResourceRef,
) -> Result<Option<SocketReference>, Error> {
    let Some(instance) = shared.instance.lock().map_err(|_| Error::Worker)?.clone() else {
        return Ok(None);
    };
    let snapshot = shared.identity.snapshot();
    Ok(Some(SocketReference {
        node_id: shared.plan.node,
        instance,
        netmap: snapshot.reference(),
        socket_id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
        socket_generation: shared.generation,
        resource,
    }))
}

/// Native UDP carriers with `peer` the plan admits, in either direction.
fn udp_carrier(shared: &EpochContext, carrier: &Carrier) -> bool {
    let peer = if carrier.dialer == shared.plan.node {
        carrier.listener
    } else {
        carrier.dialer
    };
    carrier.protocol == TransportProtocol::NativeUdp
        && shared
            .plan
            .admit(shared.identity.snapshot(), peer, carrier)
            .is_ok()
}

struct Pending {
    offer: ValidatedOffer,
    endpoint: [u8; 32],
    active: bool,
    accept_at: Instant,
}

// Capability knowledge from the latest bounded metadata resolution, not I/O
// authority. A missing STUN address family cannot make healthy sockets poll
// forever; failures remain retryable and metadata expiry forces a fresh query.
struct Observers {
    expires: Instant,
    endpoints: std::collections::BTreeSet<[u8; 32]>,
}

// Retained across epoch/resource handoffs: churn cannot reset admission.
// Acquisition is bounded to 30 starts in any half-open minute, paced at 250 ms.
const QUERY_INTERVAL: Duration = Duration::from_millis(250);
// Mapping observations expire after at most 20 seconds. Their refresh budget is
// independent of missing-adjacency acquisition, but coalesced metadata queries
// have their own two-second retry budget even if acquisition used its minute.
// Successful observations below still refresh at eight seconds; an unavailable
// metadata response must not spend an eight-second successful-observation slot.
const OBSERVATION_QUERY_INTERVAL: Duration = Duration::from_secs(2);
const OBSERVATION_REFRESH_MS: u64 = 8_000;

struct QuerySchedule {
    next: Instant,
    observation_at: Instant,
    acquisitions: std::collections::VecDeque<Instant>,
}
impl QuerySchedule {
    fn new(now: Instant) -> Self {
        Self {
            next: now,
            observation_at: now,
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
            self.observation_at = now + OBSERVATION_QUERY_INTERVAL;
        } else if acquisition && self.acquisitions.len() < 30 {
            self.acquisitions.push_back(now);
        } else {
            return false;
        }
        self.next = now + QUERY_INTERVAL;
        true
    }
}

fn needs_acquisition(shared: &EpochContext, pending: &BTreeMap<UdpTraversalId, Pending>) -> bool {
    let plan = &shared.plan;
    let Ok(links) = shared.links.lock() else {
        return false;
    };
    plan.peers().any(|peer| {
        // The selected owner retires unhealthy channels and performs authorized
        // handoff. A temporarily fenced scope is not a vacant physical slot:
        // prepare() must wait for that same owner too.
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
        plan.dialers.values().any(|dial| {
            dial.peer == peer
                && udp_carrier(
                    shared,
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
                    shared,
                    &Carrier {
                        dialer: peer,
                        listener: plan.node,
                        listener_key: key.clone(),
                        protocol,
                    },
                )
        })
    })
}

fn needs_observation(shared: &EpochContext) -> bool {
    let plan = &shared.plan;
    plan.dialers.values().any(|dial| {
        dial.parameters.transport() == TransportProtocol::NativeUdp
            && !dial.endpoints.iter().any(|endpoint| {
                endpoint.source == meshway_configuration::execution::DialEndpointSource::Configured
            })
    }) || plan.listeners.values().any(|listener| {
        matches!(listener.declaration(), meshway_transport_spec::Listener::NativeUdp(udp)
            if udp.enabled && udp.endpoints.is_empty())
    })
}

/// A socket's mapping is refreshed only while it carries a healthy Native
/// UDP channel or some wanted peer has a UDP carrier. The budget is unchanged.
fn observation_wanted(shared: &EpochContext, id: &[u8; 32]) -> bool {
    let plan = &shared.plan;
    let carrying = shared.links.lock().is_ok_and(|links| {
        links.values().any(|link| {
            link.fact.socket_id == *id
                && link.fact.carrier.protocol == TransportProtocol::NativeUdp
                && link.scope.check().is_ok()
        })
    });
    carrying
        || plan.peers().any(|peer| {
            shared.wanted(peer)
                && plan.outgoing(peer).is_ok_and(|candidates| {
                    candidates.iter().any(|(_, dial)| {
                        dial.parameters.transport() == TransportProtocol::NativeUdp
                    })
                })
        })
}

fn observation_due(shared: &EpochContext, id: &[u8; 32]) -> bool {
    if !observation_wanted(shared, id) {
        return false;
    }
    let Ok(now) = shared.identity.snapshot().now_ms() else {
        return false;
    };
    let Ok(sockets) = shared.sockets.lock() else {
        return false;
    };
    sockets.get(id).is_some_and(|socket| {
        !socket.endpoints.iter().any(|candidate| {
            matches!(
                candidate.origin,
                meshway_model::state::EndpointSource::Stun { .. }
            ) && u64::try_from(candidate.observed_at.timestamp_millis())
                .is_ok_and(|at| now < at.saturating_add(OBSERVATION_REFRESH_MS))
                && u64::try_from(candidate.expires_at.timestamp_millis())
                    .is_ok_and(|at| now.saturating_add(OBSERVATION_REFRESH_MS) < at)
        })
    })
}

/// Scheduling state owned by the traversal owner for its whole life. Only
/// these budgets and cursors cross an epoch; offers, discovery contexts and
/// observer metadata are per epoch and keep their own short deadlines.
struct Schedule {
    queries: QuerySchedule,
    /// Earliest next STUN discovery round (paced independently of queries).
    discover_at: Instant,
    /// Fair eight-peer batch cursor for prepare.
    after_peer: Option<NodeId>,
    endpoints: Vec<[u8; 32]>,
}
impl Schedule {
    fn new() -> Self {
        Self {
            queries: QuerySchedule::new(Instant::now()),
            discover_at: Instant::now(),
            after_peer: None,
            endpoints: Vec::new(),
        }
    }
    /// A changed socket set is due for discovery immediately.
    fn sockets(&mut self, endpoints: &BTreeMap<[u8; 32], Endpoint>) {
        let current: Vec<_> = endpoints.keys().copied().collect();
        if current != self.endpoints {
            self.discover_at = Instant::now();
            self.endpoints = current;
        }
    }
}

pub(super) async fn run(mut shared: Arc<EpochContext>, gateway: Arc<dyn TraversalGateway>) {
    let mut updates = shared.epochs.subscribe();
    let mut resources = shared.resource_changes.subscribe();
    let mut schedule = Schedule::new();
    loop {
        let endpoints = match shared.traversal_endpoints.lock() {
            Ok(endpoints) => endpoints.clone(),
            Err(_) => return,
        };
        schedule.sockets(&endpoints);
        run_epoch(
            shared.clone(),
            &gateway,
            &endpoints,
            &mut updates,
            &mut resources,
            &mut schedule,
        )
        .await;
        match shared.latest(&mut updates).await {
            Ok(next) => shared = next,
            Err(_) => return,
        }
    }
}

async fn run_epoch(
    shared: Arc<EpochContext>,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    updates: &mut tokio::sync::watch::Receiver<Epoch>,
    resources: &mut tokio::sync::watch::Receiver<()>,
    schedule: &mut Schedule,
) {
    let Schedule {
        queries,
        discover_at,
        after_peer,
        ..
    } = schedule;
    let scope = shared.scope(None, false);
    let mut pending: BTreeMap<UdpTraversalId, Pending> = BTreeMap::new();
    let mut retired: BTreeMap<UdpTraversalId, u64> = BTreeMap::new();
    let mut prepare_at = Instant::now();
    let mut context: Option<DiscoveryContext> = None;
    let mut observers: Option<Observers> = None;
    let work = async {
        loop {
            if shared.epoch_changed() {
                return Err(Error::Authority);
            }
            let now = shared
                .identity
                .snapshot()
                .now_ms()
                .map_err(|_| Error::Authority)?;
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
            // A selected channel suppresses acquisition, not observation of its
            // actual NAT socket. Refresh even while its physical owner hands
            // off publication scope; the Discover command still carries only
            // the new, independently checked authority.
            let acquisition = !endpoints.is_empty() && needs_acquisition(&shared, &pending);
            let observation = needs_observation(&shared)
                && endpoints.keys().any(|id| {
                    observers.as_ref().is_none_or(|known| {
                        Instant::now() >= known.expires || known.endpoints.contains(id)
                    }) && observation_due(&shared, id)
                });
            // A live discovery context already authorizes same-socket STUN.
            // Do not put another metadata RPC (and its control-owner queue)
            // ahead of refresh when no adjacency acquisition needs that query.
            // Renew metadata within the observation refresh horizon rather
            // than waiting for a periodic expiry gap. Epoch/resource handoff
            // drops this context; query admission keeps its original budget.
            let observation_query = observation
                && (acquisition
                    || context.as_ref().is_none_or(|context| {
                        context.ensure_current().is_err()
                            || now.saturating_add(OBSERVATION_REFRESH_MS) >= context.expires_at_ms()
                    }));
            if queries.admit(Instant::now(), acquisition, observation_query) {
                let query = gateway
                    .request(Operation::Query(Box::new(
                        shared.identity.snapshot().clone(),
                    )))
                    .await;
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
                    // Owner slots retain their ORIGINAL offered guard. A later read
                    // of the same offer never replaces a guard or extends a deadline.
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
            // query, replacing guards, or extending any deadline.
            if let Some(context) = context
                .as_ref()
                .filter(|context| context.ensure_current().is_ok())
            {
                if (acquisition || observation)
                    && endpoints.keys().any(|id| observation_due(&shared, id))
                    && Instant::now() >= *discover_at
                {
                    // A cancelled enqueue is not a mapping observation. Pace
                    // retries independently, and schedule the normal refresh
                    // from actual validated observation timestamps above.
                    *discover_at = Instant::now() + Duration::from_secs(2);
                    let mut available = std::collections::BTreeSet::new();
                    for (id, endpoint) in endpoints {
                        let family = shared.sockets.lock().ok().and_then(|sockets| {
                            sockets.get(id).map(|socket| socket.bound_address.is_ipv4())
                        });
                        let Some(ipv4) = family else {
                            continue;
                        };
                        let due = observation_due(&shared, id);
                        for server in context.servers() {
                            let resolved = tokio::select! { biased;
                                _ = context.invalidated() => break,
                                result = endpoint.authorization.authorize_stun(context, &server.id) => result,
                            };
                            match resolved {
                                Ok(targets) => {
                                    let targets: Vec<_> = targets
                                        .into_iter()
                                        .filter(|target| target.is_ipv4() == ipv4)
                                        .collect();
                                    if !targets.is_empty() {
                                        available.insert(*id);
                                        if due {
                                            let _ = endpoint.commands.try_send(Command::Discover {
                                                context: context.clone(),
                                                server: server.id.clone(),
                                                targets,
                                            });
                                        }
                                    }
                                }
                                Err(_) => {
                                    available.insert(*id);
                                }
                            }
                        }
                    }
                    observers = Some(Observers {
                        expires: context.expires_at().into(),
                        endpoints: available,
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
        _ = updates.changed() => {},
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

pub(super) fn matching_endpoint(shared: &EpochContext, offer: &ValidatedOffer) -> Option<[u8; 32]> {
    let sockets = shared.sockets.lock().ok()?;
    sockets.iter().find_map(|(id, fact)| {
        (fact.reference == offer.local().candidate.socket
            && socket_compatible(&shared.plan, &offer.offer().carrier, fact)
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
    let meshway_transport_spec::DialParameters::NativeUdp { options } = &dial.parameters else {
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
    listener: &meshway_transport_spec::PreparedListener,
    local: SocketAddr,
    options: &meshway_transport_spec::NativeUdpDialer,
) -> bool {
    dial_bind_matches(options, local)
        && matches!(listener.declaration(), meshway_transport_spec::Listener::NativeUdp(listener)
            if listener.enabled && listener.bind_interface.is_empty() && listener.socket == options.socket)
}

fn dial_bind_matches(options: &meshway_transport_spec::NativeUdpDialer, local: SocketAddr) -> bool {
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
        _ = offer.invalidated() => Err(Error::Authority),
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
    shared: &EpochContext,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    context: &DiscoveryContext,
    pending: &mut BTreeMap<UdpTraversalId, Pending>,
    after_peer: &mut Option<NodeId>,
) -> Result<(), Error> {
    let plan = &shared.plan;
    let now = shared
        .identity
        .snapshot()
        .now_ms()
        .map_err(|_| Error::Authority)?;
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
            if udp_carrier(shared, &carrier) {
                carriers.entry(dial.peer).or_insert(carrier);
                break;
            }
        }
    }
    // One coordinator (the logical dialer) prepares a given carrier. Both Nodes
    // can discover/accept; physical listener ownership does not change roles.
    carriers.retain(|peer, _| {
        let reverse = plan.listeners.iter().any(|(key, listener)| {
            let (protocol, enabled) = provider::declaration(listener);
            enabled
                && udp_carrier(
                    shared,
                    &Carrier {
                        dialer: *peer,
                        listener: plan.node,
                        listener_key: key.clone(),
                        protocol,
                    },
                )
        });
        (!reverse || plan.node < *peer)
            && !pending
                .values()
                .any(|pending| pending.offer.peer().candidate.socket.node_id == *peer)
            // Scope invalidation suspends I/O, not physical ownership. Let the
            // selected owner finish handoff/retirement before a new acquisition.
            && !shared.links.lock().is_ok_and(|links| links.contains_key(peer))
    });
    // Advance before the RPC, including errors/no observations. An unavailable
    // prefix must not starve other peers; the cursor survives epoch handoff.
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
                netmap_serial = context.snapshot().reference().serial.get(),
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
                // retains the exact reserved slot in this epoch's cleanup set.
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
            // Epoch/resource changes share this schedule, including a retry
            // immediately after a previous request failed or was cancelled.
            queries += usize::from(schedule.admit(now, true, false));
            assert!(!schedule.admit(now, true, false));
        }
        assert_eq!(queries, 300, "at most 30 requests/minute for ten minutes");
    }

    #[test]
    fn observation_budget_survives_saturated_acquisition_and_publication_churn() {
        let start = Instant::now();
        let mut schedule = QuerySchedule::new(start);
        for tick in 0..30 {
            assert!(schedule.admit(start + Duration::from_millis(tick * 250), true, false));
        }
        let due = start + Duration::from_secs(8);
        assert!(!schedule.admit(due, true, false));
        assert!(schedule.admit(due, false, true));
        assert!(!schedule.admit(due + Duration::from_secs(1), true, true));
        assert!(schedule.admit(due + OBSERVATION_QUERY_INTERVAL, false, true));
        assert!(schedule.admit(start + Duration::from_secs(60), true, false));
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
