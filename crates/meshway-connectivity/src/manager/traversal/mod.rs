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

mod candidates;
mod coordination;
mod rounds;
mod socket;

use candidates::{accept, prepare, reserve};
use coordination::{Coordination, needs_acquisition};
use rounds::{QuerySchedule, Round, cadence, observation_due};
use socket::socket_active;
pub(super) use socket::{listener_covers, matching_endpoint, socket_reference};

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
    /// Kept across content and resources: only what concerns a peer resets it.
    coordination: Coordination,
}
impl Schedule {
    fn new() -> Self {
        Self {
            queries: QuerySchedule::new(Instant::now()),
            rounds: BTreeMap::new(),
            after_peer: None,
            coordination: Coordination::default(),
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
        schedule.coordination.retain(&plans.borrow_and_update());
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
        coordination,
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
                coordination.ended(&shared, value.offer.peer().candidate.socket.node_id);
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
            let acquisition =
                !endpoints.is_empty() && needs_acquisition(&shared, &pending, coordination);
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
                            coordination
                                .ended(&shared, value.offer.peer().candidate.socket.node_id);
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
                        coordination.ended(&shared, value.offer.peer().candidate.socket.node_id);
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
                        coordination,
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
