//! Bounded acquisition orchestration. Endpoint commands reserve actual receiver
//! slots; HTTP work runs independently of the host packet/sample worker.
use super::*;
use crate::traversal::{Operation, Reply, TraversalGateway};
use meshway_control::connectivity::{
    AttemptScope, AttemptView, DiscoveryContext, Outcome, ValidatedOffer, prepare_request,
};
use meshway_model::{
    Sequence, UdpTraversalId,
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

pub(super) fn socket_reference(
    identity: &DataPlaneIdentity,
    id: [u8; 32],
    generation: Sequence,
    resource: ResourceRef,
) -> Result<SocketReference, Error> {
    Ok(SocketReference {
        node_id: identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?
            .node_id,
        instance: identity.snapshot().instance(),
        configuration: identity.snapshot().reference(),
        socket_id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
        socket_generation: generation,
        resource,
    })
}

struct Pending {
    offer: ValidatedOffer,
    endpoint: [u8; 32],
    active: bool,
    accept_at: Instant,
}

// Retained across execution/resource handoffs: churn cannot reset admission.
// Acquisition can use a short publication overlap immediately, without an
// unbounded poll loop: 30 starts in any half-open minute, paced at 250 ms.
const QUERY_INTERVAL: Duration = Duration::from_millis(250);
// Mapping observations expire after at most 20 seconds. Their refresh budget is
// independent of missing-adjacency acquisition, but coalesced metadata queries
// have their own two-second retry budget even if acquisition used its minute.
// Successful observations below still refresh at eight seconds; an unavailable
// metadata response must not spend an eight-second successful-observation slot.
const OBSERVATION_QUERY_INTERVAL: Duration = Duration::from_secs(2);

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

fn needs_acquisition(shared: &Shared, pending: &BTreeMap<UdpTraversalId, Pending>) -> bool {
    let Ok(cfg) = shared.identity.snapshot().configuration() else {
        return false;
    };
    let Ok(now) = shared.identity.snapshot().now_ms() else {
        return false;
    };
    let Ok(links) = shared.links.lock() else {
        return false;
    };
    cfg.peers.keys().any(|peer| {
        // The selected owner retires unhealthy channels and performs authorized
        // handoff. A temporarily fenced scope is not a vacant physical slot:
        // prepare() must wait for that same owner too. Querying during this gap
        // would turn every normal execution refresh into candidate acquisition.
        if links.contains_key(peer) {
            return false;
        }
        // The exact admitted offer already supplies this peer's acquisition
        // context. Accept rechecks its authority; another metadata query must
        // not sit ahead of the final bilateral acceptance in a short window.
        if pending
            .values()
            .any(|value| value.offer.peer().candidate.socket.node_id == *peer)
        {
            return false;
        }
        cfg.dialers.values().any(|dial| {
            dial.peer == *peer
                && dial.parameters.transport() == TransportProtocol::NativeUdp
                && admit_adjacency(
                    cfg,
                    *peer,
                    &Carrier {
                        dialer: cfg.node_id,
                        listener: *peer,
                        listener_key: dial.listener_key.clone(),
                        protocol: TransportProtocol::NativeUdp,
                    },
                    now,
                )
                .is_ok()
        }) || cfg.listeners.iter().any(|(key, listener)| {
            let (protocol, enabled) = provider::declaration(listener);
            enabled
                && protocol == TransportProtocol::NativeUdp
                && admit_adjacency(
                    cfg,
                    *peer,
                    &Carrier {
                        dialer: *peer,
                        listener: cfg.node_id,
                        listener_key: key.clone(),
                        protocol,
                    },
                    now,
                )
                .is_ok()
        })
    })
}

fn needs_observation(shared: &Shared) -> bool {
    let Ok(cfg) = shared.identity.snapshot().configuration() else {
        return false;
    };
    cfg.dialers.values().any(|dial| {
        dial.parameters.transport() == TransportProtocol::NativeUdp && dial.endpoints.is_empty()
    }) || cfg.listeners.values().any(|listener| {
        matches!(listener.declaration(), meshway_transport_spec::Listener::NativeUdp(udp)
            if udp.enabled && udp.endpoints.is_empty())
    })
}

fn observation_due(shared: &Shared, id: &[u8; 32]) -> bool {
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
                .is_ok_and(|at| now < at.saturating_add(8_000))
                && u64::try_from(candidate.expires_at.timestamp_millis())
                    .is_ok_and(|at| now.saturating_add(8_000) < at)
        })
    })
}

pub(super) async fn run(mut shared: Arc<Shared>, gateway: Arc<dyn TraversalGateway>) {
    let mut updates = shared.epochs.subscribe();
    let mut resources = shared.resource_changes.subscribe();
    let mut discover_at = Instant::now();
    let mut after_peer = None;
    let mut previous_endpoints = Vec::new();
    let mut queries = QuerySchedule::new(Instant::now());
    loop {
        let endpoints = match shared.traversal_endpoints.lock() {
            Ok(endpoints) => endpoints.clone(),
            Err(_) => return,
        };
        let current_endpoints: Vec<_> = endpoints.keys().copied().collect();
        if current_endpoints != previous_endpoints {
            discover_at = Instant::now();
            previous_endpoints = current_endpoints;
        }
        run_epoch(
            shared.clone(),
            &gateway,
            &endpoints,
            &mut updates,
            &mut resources,
            &mut discover_at,
            &mut after_peer,
            &mut queries,
        )
        .await;
        match shared.latest(&mut updates).await {
            Ok(next) => shared = next,
            Err(_) => return,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_epoch(
    shared: Arc<Shared>,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    updates: &mut tokio::sync::watch::Receiver<DataPlaneIdentity>,
    resources: &mut tokio::sync::watch::Receiver<()>,
    discover_at: &mut Instant,
    after_peer: &mut Option<NodeId>,
    queries: &mut QuerySchedule,
) {
    let scope = shared.scope(None, false);
    let mut pending: BTreeMap<UdpTraversalId, Pending> = BTreeMap::new();
    let mut retired: BTreeMap<UdpTraversalId, u64> = BTreeMap::new();
    let mut prepare_at = Instant::now();
    let mut context: Option<DiscoveryContext> = None;
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
                && endpoints.keys().any(|id| observation_due(&shared, id));
            if queries.admit(Instant::now(), acquisition, observation) {
                let query = gateway
                    .request(Operation::Query(Box::new(
                        shared.identity.snapshot().clone(),
                    )))
                    .await;
                if let Ok(Reply::Query(Outcome::Current(discovery))) = query {
                    context = Some(discovery.context);
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
            // discovery authority. Advance the original bounded operations in
            // their short shared-publication window without issuing another
            // metadata query, replacing guards, or extending any deadline.
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
                    for (id, endpoint) in endpoints {
                        if !observation_due(&shared, id) {
                            continue;
                        }
                        for server in context.servers() {
                            let resolved = tokio::select! { biased;
                                _ = context.invalidated() => break,
                                result = endpoint.authorization.authorize_stun(context, &server.id) => result,
                            };
                            if let Ok(targets) = resolved {
                                let _ = endpoint.commands.try_send(Command::Discover {
                                    context: context.clone(),
                                    server: server.id.clone(),
                                    targets,
                                });
                            }
                        }
                    }
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

pub(super) fn matching_endpoint(shared: &Shared, offer: &ValidatedOffer) -> Option<[u8; 32]> {
    let sockets = shared.sockets.lock().ok()?;
    let cfg = shared.identity.snapshot().configuration().ok()?;
    sockets.iter().find_map(|(id, fact)| {
        (fact.reference == offer.local().candidate.socket
            && socket_compatible(cfg, &offer.offer().carrier, fact)
            && fact.endpoints.iter().any(|observation| {
                fact.candidate(observation)
                    .is_ok_and(|candidate| candidate == offer.local().candidate)
                    && observation.address == offer.local().address
            }))
        .then_some(*id)
    })
}

fn socket_compatible(
    cfg: &meshway_configuration::execution::NodeConfiguration,
    carrier: &Carrier,
    fact: &SocketFact,
) -> bool {
    if cfg.node_id == carrier.listener {
        return fact.reference.resource
            == (ResourceRef::Listener {
                key: carrier.listener_key.clone(),
            });
    }
    let key = format!("{}/{}", carrier.listener, carrier.listener_key);
    let Some(dial) = cfg.dialers.get(&key) else {
        return false;
    };
    let meshway_transport_spec::DialParameters::NativeUdp { options } = &dial.parameters else {
        return false;
    };
    match &fact.reference.resource {
        ResourceRef::Dial { key: bound } => {
            *bound == key && dial_bind_matches(options, fact.bound_address)
        }
        ResourceRef::Listener { key } => cfg
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
    shared: &Shared,
    gateway: &Arc<dyn TraversalGateway>,
    endpoints: &BTreeMap<[u8; 32], Endpoint>,
    context: &DiscoveryContext,
    pending: &mut BTreeMap<UdpTraversalId, Pending>,
    after_peer: &mut Option<NodeId>,
) -> Result<(), Error> {
    let cfg = shared
        .identity
        .snapshot()
        .configuration()
        .map_err(|_| Error::Authority)?;
    let now = shared
        .identity
        .snapshot()
        .now_ms()
        .map_err(|_| Error::Authority)?;
    let mut carriers = BTreeMap::new();
    let mut candidates: Vec<_> = cfg.dialers.iter().collect();
    candidates.sort_by(|a, b| (a.1.priority, a.0).cmp(&(b.1.priority, b.0)));
    for (_, dial) in candidates {
        let carrier = Carrier {
            dialer: cfg.node_id,
            listener: dial.peer,
            listener_key: dial.listener_key.clone(),
            protocol: dial.parameters.transport(),
        };
        if carrier.protocol == TransportProtocol::NativeUdp
            && admit_adjacency(cfg, dial.peer, &carrier, now).is_ok()
        {
            carriers.entry(dial.peer).or_insert(carrier);
        }
    }
    // One coordinator (the logical dialer) prepares a given carrier. Both Nodes
    // can discover/accept; physical listener ownership does not change roles.
    carriers.retain(|peer, _| {
        let reverse = cfg.listeners.iter().any(|(key, listener)| {
            let (protocol, enabled) = provider::declaration(listener);
            enabled
                && protocol == TransportProtocol::NativeUdp
                && admit_adjacency(
                    cfg,
                    *peer,
                    &Carrier {
                        dialer: *peer,
                        listener: cfg.node_id,
                        listener_key: key.clone(),
                        protocol,
                    },
                    now,
                )
                .is_ok()
        });
        (!reverse || cfg.node_id < *peer)
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
                        || !socket_compatible(cfg, carrier, fact)
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
                publication = %context.snapshot().reference().execution_publication,
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
