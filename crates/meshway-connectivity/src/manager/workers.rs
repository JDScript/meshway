use super::*;
use crate::ListenerSocket;

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

/// The peer's ordered dial candidates for this epoch, built once per
/// acquisition sweep from the plan's carrier order (never by planning every
/// peer), plus the peer's current fact endpoints for a netmap.
pub(super) fn candidates(shared: &EpochContext, peer: NodeId) -> Result<Vec<Candidate>, Error> {
    let plan = &shared.plan;
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
        // Resource plans survive compatible publication handoffs, including a
        // diagnostic-only or expired business grant gaining a live permit.
        // Every actual acquisition below still checks its frozen peer Scope.
        // Unsupported dial candidates cannot be used; another configured stream
        // candidate may still be usable. Enabled unsupported listeners fail start.
        let (driver, timeout) = match &candidate.parameters {
            meshway_transport_spec::DialParameters::NativeUdp { options } if options.enabled => (
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

pub(super) async fn dial(
    mut shared: Arc<EpochContext>,
    factory: Arc<dyn SocketFactory>,
    peer: NodeId,
) {
    let mut updates = shared.epochs.subscribe();
    loop {
        dial_epoch(shared.clone(), factory.clone(), peer).await;
        match shared.latest(&mut updates).await {
            Ok(next) => shared = next,
            Err(_) => return,
        }
    }
}

/// Wait until this peer is wanted (interest or keepalive), or the epoch ends.
async fn wanted(
    shared: &EpochContext,
    lifetime: &Scope,
    peer: NodeId,
    updates: &mut tokio::sync::watch::Receiver<Epoch>,
) -> Result<(), Error> {
    let wake = shared.interests.waker(peer);
    loop {
        let notified = wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if shared.wanted(peer) {
            return Ok(());
        }
        lifetime
            .run(async {
                if shared.epoch_changed() {
                    return Err(Error::Authority);
                }
                tokio::select! { biased;
                    _ = updates.changed() => Err(Error::Authority),
                    _ = &mut notified => Ok(()),
                }
            })
            .await?;
    }
}

async fn dial_epoch(shared: Arc<EpochContext>, factory: Arc<dyn SocketFactory>, peer: NodeId) {
    let lifetime = shared.scope(None, false);
    let mut updates = shared.epochs.subscribe();
    'acquisition: loop {
        if lifetime.check().is_err() || shared.epoch_changed() {
            return;
        }
        if wanted(&shared, &lifetime, peer, &mut updates)
            .await
            .is_err()
        {
            return;
        }
        // A revoked operation scope is not a vacant physical slot. During a
        // direct handoff its owner must first renew or retire the selected link;
        // racing that decision with a reverse dial can replace a retained FD.
        let selected = shared
            .links
            .lock()
            .is_ok_and(|links| links.contains_key(&peer));
        if selected {
            if backoff(&shared, &lifetime, peer, &mut updates)
                .await
                .is_err()
            {
                return;
            }
            continue;
        }
        let Ok(candidates) = candidates(&shared, peer) else {
            return;
        };
        // Provenance is the first priority across all of this peer's carriers.
        // In particular, an unreachable local UDP address must not delay an
        // explicitly configured TCP fallback after a coordinated link fails.
        'candidates: for source in [
            LinkAcquisition::PublishedEndpoint,
            LinkAcquisition::LocalEndpoint,
        ] {
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
                        break 'candidates;
                    }
                    if matches!(&candidate.driver, Driver::Datagram) {
                        let _ = datagram::dial(
                            shared.clone(),
                            factory.clone(),
                            peer,
                            &candidate.key,
                            &candidate.carrier,
                            target,
                            candidate.timeout,
                        )
                        .await;
                        if lifetime.check().is_err() {
                            return;
                        }
                        if backoff(&shared, &lifetime, peer, &mut updates)
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
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
                            .connect_endpoint(
                                shared.identity.snapshot(),
                                &candidate.key,
                                &target.endpoint,
                            )
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
                        let (stream, id) =
                            adjacency::dial(&shared.identity, &scope, stream, &candidate.carrier)
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
                            },
                            opening,
                        ))
                    };
                    let result = tokio::select! { biased;
                        _ = updates.changed() => {
                            scope.cancellation.cancel();
                            return;
                        },
                        result = scope.run(attempt) => result,
                    };
                    let mut was_selected = false;
                    if let Ok((stream, fact, opening)) = result {
                        let _ = connected(shared.clone(), stream, fact, opening, &mut was_selected)
                            .await;
                    }
                    drop(capacity);
                    if lifetime.check().is_err() {
                        return;
                    }
                    // Every failure/rejected duplicate is paced; no immediate
                    // failed-dial loop, including single-candidate publications.
                    if backoff(&shared, &lifetime, peer, &mut updates)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if was_selected {
                        // A selected stream ending starts a new acquisition,
                        // not the untried suffix of its old candidate sweep.
                        // In particular, retry configured TCP before a local
                        // UDP blackhole after asymmetric peer health expiry.
                        continue 'acquisition;
                    }
                }
            }
        }
        if backoff(&shared, &lifetime, peer, &mut updates)
            .await
            .is_err()
        {
            return;
        }
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

/// Paced retry. New peer facts end the wait early (a changed candidate set
/// may be dialable now); interest alone never shortens the pace.
async fn backoff(
    shared: &EpochContext,
    lifetime: &Scope,
    peer: NodeId,
    updates: &mut tokio::sync::watch::Receiver<Epoch>,
) -> Result<(), Error> {
    let facts = shared.facts.get(peer);
    let wake = shared.interests.waker(peer);
    let deadline = tokio::time::Instant::now() + shared.limits.retry_backoff;
    lifetime
        .run(async {
            loop {
                if shared.epoch_changed() {
                    return Err(Error::Authority);
                }
                tokio::select! { biased;
                    _ = updates.changed() => return Err(Error::Authority),
                    _ = tokio::time::sleep_until(deadline) => return Ok(()),
                    _ = wake.notified() => {
                        if shared.facts.get(peer) != facts {
                            return Ok(());
                        }
                    },
                }
            }
        })
        .await
}

pub(super) async fn listen(
    mut shared: Arc<EpochContext>,
    fact: ListenerFact,
    listener: Box<dyn ListenerSocket>,
    acceptor: Arc<provider::Acceptor>,
) {
    let mut updates = shared.epochs.subscribe();
    let mut children = JoinSet::new();
    loop {
        let lifetime = shared.scope(None, false);
        let socket = lifetime
            .run(async {
                if shared.epoch_changed() {
                    return Err(Error::Authority);
                }
                tokio::select! { biased;
                    _ = updates.changed() => Err(Error::Authority),
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
            Err(_) if shared.stop.is_cancelled() => break,
            Err(_)
                if shared.identity.snapshot().ensure_current().is_err()
                    || shared.epoch_changed() =>
            {
                match shared.successor(&mut updates).await {
                    Ok(next) => {
                        shared = next;
                        continue;
                    }
                    Err(_) => break,
                }
            }
            Err(_) => {
                shared.stop.cancel();
                break;
            }
        };
        let Ok(capacity) = shared.capacity.clone().try_acquire_owned() else {
            continue;
        };
        // Only the claimed dialer's carrier is admitted (and cached per
        // epoch); the full context and signature are verified against it.
        let plan = shared.plan.clone();
        let snapshot = shared.identity.snapshot().clone();
        let key = fact.key.clone();
        let protocol = fact.protocol;
        let carriers = move |peer| plan.accepting(&snapshot, peer, &key, protocol);
        let shared = shared.clone();
        let acceptor = acceptor.clone();
        children.spawn(async move {
            let _capacity = capacity;
            let scope = shared.scope(None, true);
            let attempt = async {
                let opening = Arc::new(scope::Opening::new(scope.clone()));
                let raw = Box::new(scope::Stream {
                    inner: socket.stream,
                    scope: shared.physical()?,
                    opening: Some(opening.clone()),
                });
                let stream = acceptor.accept(raw).await?;
                let (stream, peer, carrier, id) =
                    adjacency::accept(&shared.identity, &scope, stream, carriers).await?;
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
                    },
                    opening,
                ))
            };
            if let Ok((stream, fact, opening)) = scope.run(attempt).await {
                let mut was_selected = false;
                let _ = connected(shared, stream, fact, opening, &mut was_selected).await;
            }
        });
    }
    drop(listener);
    shared.stop.cancel();
    while children.join_next().await.is_some() {}
}
