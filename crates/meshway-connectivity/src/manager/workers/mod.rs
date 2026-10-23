use super::*;
use crate::ListenerSocket;

mod pacing;
use pacing::Pacing;
pub(super) use pacing::backoff;
#[cfg(test)]
use pacing::{MAX_CANDIDATE_BACKOFF, TargetKey};

pub(super) struct Candidate {
    key: String,
    carrier: Carrier,
    endpoints: Vec<Target>,
    driver: Driver,
    timeout: std::time::Duration,
}

/// One endpoint to dial: configured in the candidate or from peer facts.
#[derive(Clone)]
pub(super) struct Target {
    pub endpoint: meshway_configuration::execution::DialEndpoint,
    pub acquisition: LinkAcquisition,
}

enum Driver {
    Stream(Box<provider::Connector>),
    Datagram,
}

/// The peer's ordered dial candidates on `plan`, built once per acquisition
/// sweep from its carrier order (never by planning every peer), plus the
/// peer's current fact endpoints.
pub(super) fn candidates(
    shared: &Shared,
    plan: &Plan,
    peer: NodeId,
) -> Result<Vec<Candidate>, Error> {
    let facts = shared.facts.get(peer);
    let now = chrono::Utc::now().timestamp_millis();
    let mut out = Vec::new();
    for (key, candidate) in plan.outgoing(peer)? {
        let carrier = Carrier {
            dialer: plan.node,
            listener: candidate.peer,
            listener_key: candidate.listener_key.clone(),
            protocol: candidate.parameters.transport(),
        };
        // Unsupported dial candidates cannot be used; another configured stream
        // candidate may still be usable. Enabled unsupported listeners fail start.
        let (driver, timeout) = match &candidate.parameters {
            meshway_transport::spec::DialParameters::NativeUdp { options } if options.enabled => (
                Driver::Datagram,
                options
                    .connect_timeout
                    .as_std()
                    .min(shared.limits.handshake_timeout),
            ),
            parameters => match provider::Connector::new(parameters) {
                Ok(connector) => (
                    Driver::Stream(Box::new(connector)),
                    shared.limits.handshake_timeout,
                ),
                Err(_) => continue,
            },
        };
        let mut endpoints: Vec<_> = candidate
            .endpoints
            .iter()
            .map(|endpoint| Target {
                endpoint: endpoint.clone(),
                acquisition: acquisition(endpoint.source),
            })
            .collect();
        if let Some(facts) = &facts {
            let network = if carrier.protocol == TransportProtocol::NativeUdp {
                meshway_model::state::SocketNetwork::Udp
            } else {
                meshway_model::state::SocketNetwork::Tcp
            };
            for endpoint in crate::plan::fact_endpoints(facts, &carrier.listener_key, network, now)
            {
                if !endpoints.iter().any(|known| {
                    known.endpoint.host == endpoint.host && known.endpoint.port == endpoint.port
                }) {
                    endpoints.push(Target {
                        acquisition: acquisition(endpoint.source),
                        endpoint,
                    });
                }
            }
        }
        if endpoints.is_empty() {
            continue;
        }
        out.push(Candidate {
            key: key.clone(),
            carrier,
            endpoints,
            driver,
            timeout,
        });
    }
    Ok(out)
}

/// Endpoint sources an explicit dial tries, in this priority order. STUN
/// mappings are tried only by coordinated traversal.
const DIALED: [LinkAcquisition; 2] = [
    LinkAcquisition::PublishedEndpoint,
    LinkAcquisition::LocalEndpoint,
];

/// This Node's own public mappings as its sockets last observed them. A new
/// mapping (another egress address or port, e.g. a VPN changing its public
/// IP) is a new network path: earlier failures say nothing about it.
pub(super) fn own_mappings(shared: &Shared) -> std::collections::BTreeSet<String> {
    shared
        .sockets
        .lock()
        .map(|sockets| {
            sockets
                .values()
                .flat_map(|socket| {
                    socket
                        .endpoints
                        .iter()
                        .map(|endpoint| endpoint.address.clone())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn last_link(shared: &Shared, peer: NodeId) -> Option<[u8; 32]> {
    shared
        .links
        .lock()
        .ok()
        .and_then(|links| links.last.get(&peer).copied())
}

/// One peer's acquisition owner for the Manager's lifetime. Every sweep
/// reads the current plan; Manager's apply stops this owner when the peer is
/// no longer a dial peer.
pub(super) async fn dial(shared: Arc<Shared>, factory: Arc<dyn SocketFactory>, peer: NodeId) {
    let lifetime = shared.scope(None, false);
    let mut plans = shared.plans.subscribe();
    let mut pacing = Pacing::default();
    // Attempts at an earlier Native UDP carrier beside a selected stream
    // channel; its own ledger, so a selected channel does not reset it.
    let mut upgrades = Pacing::default();
    // The peer facts and candidates the selected channel was last checked against.
    let mut moved_from = shared.facts.get(peer);
    let mut candidate_targets = dialed_targets(&shared, &shared.plan(), peer);
    'acquisition: loop {
        if lifetime.check().is_err() {
            return;
        }
        if wanted(&shared, &lifetime, peer).await.is_err() {
            return;
        }
        // A selected channel is not a vacant physical slot; only its own
        // owner, idle retirement or Manager's apply ends it.
        let selected = shared
            .links
            .lock()
            .is_ok_and(|links| links.contains_key(&peer));
        if selected {
            let facts = shared.facts.get(peer);
            if moved_from != facts {
                moved_from = facts;
                moved(&shared, &factory, peer, &mut candidate_targets).await;
                if lifetime.check().is_err() {
                    return;
                }
            }
            pacing.connected(last_link(&shared, peer));
            beside(&shared, &factory, peer, &mut upgrades).await;
            if lifetime.check().is_err() {
                return;
            }
            if pause(&shared, &lifetime, peer, None).await.is_err() {
                return;
            }
            continue;
        }
        plans.borrow_and_update();
        let plan = shared.plan();
        let Ok(candidates) = candidates(&shared, &plan, peer) else {
            return;
        };
        let restarted = pacing.sweep(
            plan.edge(peer),
            candidates
                .iter()
                .flat_map(|candidate| {
                    candidate
                        .endpoints
                        .iter()
                        .filter(|target| DIALED.contains(&target.acquisition))
                        .map(|target| Pacing::key(candidate, target))
                })
                .collect(),
            own_mappings(&shared),
            shared.roam_epoch(),
            last_link(&shared, peer),
        );
        if let Some(cause) = restarted
            && diagnostic_event()
        {
            tracing::debug!(operation = "candidate_pacing", peer = %peer, cause,
                "Connectivity candidate pacing starts over");
        }
        // Provenance is the first priority across all of this peer's carriers.
        // In particular, an unreachable local UDP address must not delay an
        // explicitly configured TCP fallback after a coordinated link fails.
        for source in DIALED {
            for candidate in candidates.iter() {
                for target in candidate
                    .endpoints
                    .iter()
                    .filter(|target| target.acquisition == source)
                {
                    if !shared.wanted(peer) {
                        continue 'acquisition;
                    }
                    if shared
                        .links
                        .lock()
                        .is_ok_and(|links| links.contains_key(&peer))
                    {
                        continue 'acquisition;
                    }
                    let key = Pacing::key(candidate, target);
                    if !pacing.due(&key, tokio::time::Instant::now()) {
                        continue;
                    }
                    let connected = if matches!(&candidate.driver, Driver::Datagram) {
                        // The dial owner serves its channel for the channel's
                        // life; meanwhile the peer's listener may move.
                        let primary = datagram::dial(
                            shared.clone(),
                            factory.clone(),
                            plan.clone(),
                            peer,
                            &candidate.key,
                            &candidate.carrier,
                            target,
                            candidate.timeout,
                            None,
                        );
                        tokio::pin!(primary);
                        let mut replacement: Option<
                            std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
                        > = None;
                        let mut served = false;
                        let wake = shared.interests.waker(peer);
                        loop {
                            let notified = wake.notified();
                            tokio::select! { biased;
                                _ = &mut primary, if !served => {
                                    served = true;
                                    if replacement.is_none() { break; }
                                },
                                () = async { if let Some(attempt) = replacement.as_mut() { attempt.await } }, if replacement.is_some() => {
                                    replacement = None;
                                    if served { break; }
                                },
                                _ = notified, if replacement.is_none() && !served => {
                                    let facts = shared.facts.get(peer);
                                    if moved_from != facts {
                                        moved_from = facts;
                                        let (shared, factory) = (shared.clone(), factory.clone());
                                        let mut previous = candidate_targets.clone();
                                        candidate_targets = dialed_targets(&shared, &shared.plan(), peer);
                                        replacement = Some(Box::pin(async move {
                                            moved(&shared, &factory, peer, &mut previous).await;
                                        }));
                                    }
                                },
                            }
                        }
                        if lifetime.check().is_err() {
                            return;
                        }
                        // Its channel, or one of another owner, was installed
                        // meanwhile (and may already have ended).
                        last_link(&shared, peer) != pacing.last_link
                    } else {
                        let scope = shared.scope(Some((peer, candidate.carrier.clone())), true);
                        if scope.check().is_err() {
                            continue;
                        }
                        let Ok(capacity) = shared.capacity.clone().try_acquire_owned() else {
                            break;
                        };
                        let attempt = async {
                            let opening = Arc::new(scope::Opening::new(scope.clone()));
                            let socket = factory
                                .connect_endpoint(&plan.desired, &candidate.key, &target.endpoint)
                                .await?;
                            let raw = Box::new(scope::Stream {
                                inner: socket.stream,
                                scope: shared.physical()?,
                                opening: Some(opening.clone()),
                            });
                            let Driver::Stream(connector) = &candidate.driver else {
                                return Err(Error::Configuration);
                            };
                            let stream = connector.connect(raw).await?;
                            let (stream, id, peer_boot) = adjacency::dial(
                                &shared.keys,
                                &plan,
                                &scope,
                                stream,
                                &candidate.carrier,
                                &shared.boot,
                            )
                            .await?;
                            Ok((
                                stream,
                                LinkFact {
                                    id,
                                    socket_id: id,
                                    peer,
                                    carrier: candidate.carrier.clone(),
                                    local: socket.local,
                                    remote: socket.peer,
                                    acquisition: target.acquisition.clone(),
                                    peer_boot,
                                },
                                opening,
                            ))
                        };
                        let result = tokio::select! { biased;
                            // New content: restart the sweep on it.
                            _ = plans.changed() => {
                                scope.cancellation.cancel();
                                continue 'acquisition;
                            },
                            result = scope.run(attempt) => result,
                        };
                        let mut was_selected = false;
                        if let Ok((stream, fact, opening)) = result {
                            // This owner serves the stream for its life; an
                            // earlier Native UDP carrier is tried beside it.
                            // A channel that replaced the stream is served by
                            // `beside` until it ends too.
                            let served = connected(
                                shared.clone(),
                                stream,
                                fact,
                                plan.clone(),
                                opening,
                                &mut was_selected,
                            );
                            tokio::pin!(served);
                            let upgrading = beside(&shared, &factory, peer, &mut upgrades);
                            tokio::pin!(upgrading);
                            let (mut stream_done, mut upgrade_done) = (false, false);
                            while !(stream_done && upgrade_done) {
                                tokio::select! { biased;
                                    _ = &mut served, if !stream_done => stream_done = true,
                                    () = &mut upgrading, if !upgrade_done => upgrade_done = true,
                                }
                            }
                        }
                        drop(capacity);
                        if lifetime.check().is_err() {
                            return;
                        }
                        was_selected || last_link(&shared, peer) != pacing.last_link
                    };
                    if connected {
                        // A channel that ended starts a new acquisition, not
                        // the untried suffix of its old sweep: in particular,
                        // configured TCP is retried before a local UDP
                        // blackhole after asymmetric peer health expiry. One
                        // pace keeps a channel that fails at once from looping.
                        pacing.connected(last_link(&shared, peer));
                        if pause(&shared, &lifetime, peer, None).await.is_err() {
                            return;
                        }
                        continue 'acquisition;
                    }
                    let (failures, retry) = pacing.failed(
                        key,
                        shared.limits.retry_backoff,
                        tokio::time::Instant::now(),
                    );
                    if diagnostic_event() {
                        tracing::debug!(operation = "candidate_backoff", peer = %peer,
                            candidate = %candidate.key, endpoint = %target.endpoint.host,
                            port = target.endpoint.port.get(), failures,
                            retry_ms = retry.as_millis() as u64,
                            "Connectivity candidate failed");
                    }
                }
            }
        }
        if pause(&shared, &lifetime, peer, pacing.next_due())
            .await
            .is_err()
        {
            return;
        }
    }
}

/// While the selected channel with `peer` is a stream although the edge's
/// carrier order puts a Native UDP candidate this Node dials before it
/// (typically after a transient outage in which the stream was acquired
/// first), dial that candidate beside the channel on `pacing`'s own back-off.
/// One that authenticates replaces exactly that channel, the same path as
/// [`moved`]; established circuits keep their full path across the handover,
/// and this owner serves the new channel until it ends. A failed attempt
/// leaves the stream as it is. Returns at once when no upgrade applies, and
/// when the stream ends.
async fn beside(
    shared: &Arc<Shared>,
    factory: &Arc<dyn SocketFactory>,
    peer: NodeId,
    pacing: &mut Pacing,
) {
    let mut plans = shared.plans.subscribe();
    loop {
        plans.borrow_and_update();
        let Some((link, ended)) = shared.links.lock().ok().and_then(|links| {
            links
                .get(&peer)
                .filter(|link| link.scope.check().is_ok())
                .map(|link| (link.fact.clone(), link.scope.cancellation.clone()))
        }) else {
            return;
        };
        let plan = shared.plan();
        let Ok(candidates) = candidates(shared, &plan, peer) else {
            return;
        };
        let earlier = earlier_datagram(&candidates, link.carrier.protocol);
        if earlier.is_empty() {
            return;
        }
        let targets = earlier
            .iter()
            .flat_map(|candidate| {
                candidate
                    .endpoints
                    .iter()
                    .filter(|target| DIALED.contains(&target.acquisition))
                    .map(|target| Pacing::key(candidate, target))
            })
            .collect();
        pacing.sweep(
            plan.edge(peer),
            targets,
            own_mappings(shared),
            shared.roam_epoch(),
            Some(link.id),
        );
        for source in DIALED {
            for candidate in &earlier {
                for target in candidate
                    .endpoints
                    .iter()
                    .filter(|target| target.acquisition == source)
                {
                    let key = Pacing::key(candidate, target);
                    if !pacing.due(&key, tokio::time::Instant::now()) {
                        continue;
                    }
                    if ended.is_cancelled() {
                        return;
                    }
                    let _ = datagram::dial(
                        shared.clone(),
                        factory.clone(),
                        plan.clone(),
                        peer,
                        &candidate.key,
                        &candidate.carrier,
                        target,
                        candidate.timeout,
                        Some(link.id),
                    )
                    .await;
                    if last_link(shared, peer) != Some(link.id) {
                        // Its channel replaced the stream and has ended
                        // since, or another owner's did.
                        if diagnostic_event() {
                            tracing::debug!(operation = "carrier_upgrade", peer = %peer,
                                candidate = %candidate.key, from = ?link.carrier.protocol,
                                "Connectivity selected stream replaced by an earlier Native UDP carrier");
                        }
                        return;
                    }
                    let (failures, retry) = pacing.failed(
                        key,
                        shared.limits.retry_backoff,
                        tokio::time::Instant::now(),
                    );
                    if diagnostic_event() {
                        tracing::debug!(operation = "carrier_upgrade", peer = %peer,
                            candidate = %candidate.key, endpoint = %target.endpoint.host,
                            port = target.endpoint.port.get(), from = ?link.carrier.protocol,
                            failures, retry_ms = retry.as_millis() as u64,
                            "Connectivity earlier carrier not acquired; the selected stream stays");
                    }
                }
            }
        }
        let due = pacing
            .next_due()
            .unwrap_or_else(|| tokio::time::Instant::now() + shared.limits.retry_backoff);
        tokio::select! { biased;
            () = ended.cancelled() => return,
            changed = plans.changed() => if changed.is_err() { return },
            () = tokio::time::sleep_until(due) => {},
        }
    }
}

/// The Native UDP candidates ordered before every candidate of `selected`'s
/// protocol in the edge's carrier order. Only a stream is replaced: between
/// Native UDP candidates and between streams the selected channel stays.
fn earlier_datagram(candidates: &[Candidate], selected: TransportProtocol) -> Vec<&Candidate> {
    if selected == TransportProtocol::NativeUdp {
        return Vec::new();
    }
    let Some(position) = candidates
        .iter()
        .position(|candidate| candidate.carrier.protocol == selected)
    else {
        return Vec::new();
    };
    candidates[..position]
        .iter()
        .filter(|candidate| matches!(candidate.driver, Driver::Datagram))
        .collect()
}

/// The literal addresses of the peer's explicitly dialed candidates (named
/// endpoints are not compared).
fn dialed_targets(
    shared: &Shared,
    plan: &Plan,
    peer: NodeId,
) -> std::collections::BTreeSet<SocketAddr> {
    candidates(shared, plan, peer)
        .unwrap_or_default()
        .iter()
        .flat_map(|candidate| candidate.endpoints.iter())
        .filter(|target| DIALED.contains(&target.acquisition))
        .filter_map(|target| {
            let ip = target.endpoint.host.parse::<std::net::IpAddr>().ok()?;
            Some(SocketAddr::new(ip, target.endpoint.port.get()))
        })
        .collect()
}

/// The peer's listener moved: the selected Native UDP channel's remote was
/// one of the peer's published candidates and its facts no longer publish
/// it, but publish others for the channel's carrier. Dial those beside the
/// channel; one that authenticates replaces exactly that channel. A remote
/// that never was a candidate (a mapping the channel followed by
/// return-routability) is not a reason to dial. `previous` holds the
/// candidates seen at the last check.
async fn moved(
    shared: &Arc<Shared>,
    factory: &Arc<dyn SocketFactory>,
    peer: NodeId,
    previous: &mut std::collections::BTreeSet<SocketAddr>,
) {
    let plan = shared.plan();
    let current = dialed_targets(shared, &plan, peer);
    let before = std::mem::replace(previous, current);
    let Some(link) = shared.links.lock().ok().and_then(|links| {
        links
            .get(&peer)
            .filter(|link| {
                link.scope.check().is_ok()
                    && link.fact.carrier.protocol == TransportProtocol::NativeUdp
                    && link.fact.carrier.dialer == plan.node
            })
            .map(|link| link.fact.clone())
    }) else {
        return;
    };
    let Ok(candidates) = candidates(shared, &plan, peer) else {
        return;
    };
    let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.carrier == link.carrier)
    else {
        return;
    };
    let targets: Vec<_> = candidate
        .endpoints
        .iter()
        .filter(|target| DIALED.contains(&target.acquisition))
        .collect();
    let literal: Vec<Option<SocketAddr>> = targets
        .iter()
        .map(|target| {
            target
                .endpoint
                .host
                .parse::<std::net::IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, target.endpoint.port.get()))
        })
        .collect();
    if targets.is_empty()
        || !before.contains(&link.remote)
        || literal.iter().any(Option::is_none)
        || literal.contains(&Some(link.remote))
    {
        return;
    }
    if diagnostic_event() {
        tracing::debug!(operation = "roam", cause = "peer_candidates_moved", peer = %peer,
            channel = ?link.id, remote = %link.remote, candidates = targets.len(),
            "Connectivity roam");
    }
    for target in targets {
        let _ = datagram::dial(
            shared.clone(),
            factory.clone(),
            plan.clone(),
            peer,
            &candidate.key,
            &candidate.carrier,
            target,
            candidate.timeout,
            Some(link.id),
        )
        .await;
        if shared.links.lock().is_ok_and(|links| {
            links
                .get(&peer)
                .is_none_or(|current| current.fact.id != link.id)
        }) {
            return;
        }
    }
}

/// Wait until this peer is wanted (interest or resident), or the owner stops.
async fn wanted(shared: &Shared, lifetime: &Scope, peer: NodeId) -> Result<(), Error> {
    let wake = shared.interests.waker(peer);
    let mut plans = shared.plans.subscribe();
    loop {
        let notified = wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if shared.wanted(peer) {
            return Ok(());
        }
        lifetime
            .run(async {
                // New content may make the peer resident.
                tokio::select! { biased;
                    result = plans.changed() => result.map_err(|_| Error::Cancelled),
                    _ = &mut notified => Ok(()),
                }
            })
            .await?;
    }
}

pub(super) fn acquisition(
    source: meshway_configuration::execution::DialEndpointSource,
) -> LinkAcquisition {
    match source {
        meshway_configuration::execution::DialEndpointSource::Configured => {
            LinkAcquisition::PublishedEndpoint
        }
        meshway_configuration::execution::DialEndpointSource::Local => {
            LinkAcquisition::LocalEndpoint
        }
        meshway_configuration::execution::DialEndpointSource::Stun => LinkAcquisition::StunEndpoint,
    }
}

/// Wait until `until` (or one `retry_backoff`). New content, changed peer
/// facts or a channel installed meanwhile end the wait early: the candidate
/// set may have changed. Interest alone never shortens it.
async fn pause(
    shared: &Shared,
    lifetime: &Scope,
    peer: NodeId,
    until: Option<tokio::time::Instant>,
) -> Result<(), Error> {
    let facts = shared.facts.get(peer);
    let link = last_link(shared, peer);
    let wake = shared.interests.waker(peer);
    let mut plans = shared.plans.subscribe();
    // A path change of this Node is a new network: try again at once.
    let mut roams = shared.roams.subscribe();
    roams.borrow_and_update();
    let deadline =
        until.unwrap_or_else(|| tokio::time::Instant::now() + shared.limits.retry_backoff);
    lifetime
        .run(async {
            loop {
                tokio::select! { biased;
                    result = plans.changed() => return result.map_err(|_| Error::Cancelled),
                    result = roams.changed() => return result.map_err(|_| Error::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => return Ok(()),
                    _ = wake.notified() => {
                        if shared.facts.get(peer) != facts || last_link(shared, peer) != link {
                            return Ok(());
                        }
                    },
                }
            }
        })
        .await
}

pub(super) async fn listen(
    shared: Arc<Shared>,
    fact: ListenerFact,
    listener: Box<dyn ListenerSocket>,
    acceptor: Arc<provider::Acceptor>,
) {
    let mut children = JoinSet::new();
    loop {
        let lifetime = shared.scope(None, false);
        let socket = lifetime
            .run(async {
                tokio::select! { biased;
                    result = children.join_next(), if !children.is_empty() => {
                        if result.is_some_and(|result| result.is_err()) {
                            shared.stop.cancel();
                            return Err(Error::Worker);
                        }
                        Ok(None)
                    },
                    socket = listener.accept() => match socket {
                        Ok(socket) => Ok(Some(socket)),
                        Err(_) => Err(Error::Underlay),
                    },
                }
            })
            .await;
        let socket = match socket {
            Ok(Some(socket)) => socket,
            Ok(None) => continue,
            Err(_) => {
                shared.stop.cancel();
                break;
            }
        };
        let Ok(capacity) = shared.capacity.clone().try_acquire_owned() else {
            continue;
        };
        // Only the claimed dialer's carrier on the current content is
        // admitted (and cached per content); the full context and signature
        // are verified against it.
        let plan = shared.plan();
        let key = fact.key.clone();
        let protocol = fact.protocol;
        let shared = shared.clone();
        let acceptor = acceptor.clone();
        children.spawn(async move {
            let _capacity = capacity;
            let scope = shared.scope(None, true);
            let carriers = {
                let plan = plan.clone();
                move |peer| plan.accepting(peer, &key, protocol)
            };
            let attempt = async {
                let opening = Arc::new(scope::Opening::new(scope.clone()));
                let raw = Box::new(scope::Stream {
                    inner: socket.stream,
                    scope: shared.physical()?,
                    opening: Some(opening.clone()),
                });
                let stream = acceptor.accept(raw).await?;
                let (stream, peer, carrier, id, peer_boot) =
                    adjacency::accept(&shared.keys, &plan, &scope, stream, carriers, &shared.boot)
                        .await?;
                Ok((
                    stream,
                    LinkFact {
                        id,
                        socket_id: id,
                        peer,
                        carrier,
                        local: socket.local,
                        remote: socket.peer,
                        acquisition: LinkAcquisition::AcceptedInbound,
                        peer_boot,
                    },
                    opening,
                ))
            };
            if let Ok((stream, fact, opening)) = scope.run(attempt).await {
                let mut was_selected = false;
                let _ = connected(shared, stream, fact, plan, opening, &mut was_selected).await;
            }
        });
    }
    drop(listener);
    shared.stop.cancel();
    while children.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests;
