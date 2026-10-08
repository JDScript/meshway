use super::*;
use crate::ListenerSocket;

pub(super) struct Candidate {
    key: String,
    carrier: Carrier,
    endpoints: Vec<LinkAcquisition>,
    driver: Driver,
    priority: u16,
    timeout: std::time::Duration,
}

enum Driver {
    Stream(Box<provider::Connector>),
    Datagram,
}

pub(super) fn plans(
    identity: &DataPlaneIdentity,
    limits: Limits,
) -> Result<BTreeMap<NodeId, Vec<Candidate>>, Error> {
    let cfg = identity
        .snapshot()
        .configuration()
        .map_err(|_| Error::Authority)?;
    let mut plans: BTreeMap<NodeId, Vec<Candidate>> = BTreeMap::new();
    for (key, candidate) in &cfg.dialers {
        let carrier = Carrier {
            dialer: cfg.node_id,
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
                    .min(limits.handshake_timeout),
            ),
            parameters => match provider::Connector::new(parameters) {
                Ok(connector) => (
                    Driver::Stream(Box::new(connector)),
                    limits.handshake_timeout,
                ),
                Err(_) => continue,
            },
        };
        if carrier.listener_key.is_empty() || carrier.listener_key.len() > 128 {
            return Err(Error::Configuration);
        }
        if candidate.endpoints.is_empty() {
            continue;
        }
        plans.entry(candidate.peer).or_default().push(Candidate {
            key: key.clone(),
            carrier,
            endpoints: candidate
                .endpoints
                .iter()
                .map(|endpoint| acquisition(endpoint.source))
                .collect(),
            driver,
            priority: candidate.priority,
            timeout,
        });
    }
    for candidates in plans.values_mut() {
        candidates.sort_by(|a, b| (a.priority, &a.key).cmp(&(b.priority, &b.key)));
    }
    Ok(plans)
}

pub(super) async fn dial(mut shared: Arc<Shared>, factory: Arc<dyn SocketFactory>, peer: NodeId) {
    let mut updates = shared.epochs.subscribe();
    loop {
        let candidates = match plans(&shared.identity, shared.limits) {
            Ok(mut plans) => Arc::new(plans.remove(&peer).unwrap_or_default()),
            Err(_) => return,
        };
        dial_epoch(shared.clone(), factory.clone(), peer, candidates).await;
        match shared.latest(&mut updates).await {
            Ok(next) => shared = next,
            Err(_) => return,
        }
    }
}

async fn dial_epoch(
    shared: Arc<Shared>,
    factory: Arc<dyn SocketFactory>,
    peer: NodeId,
    candidates: Arc<Vec<Candidate>>,
) {
    let lifetime = shared.scope(None, false);
    let mut updates = shared.epochs.subscribe();
    'acquisition: loop {
        if lifetime.check().is_err() || shared.epoch_changed() {
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
            if backoff(&shared, &lifetime, &mut updates).await.is_err() {
                return;
            }
            continue;
        }
        // Provenance is the first priority across all of this peer's carriers.
        // In particular, an unreachable local UDP address must not delay an
        // explicitly configured TCP fallback after a coordinated link fails.
        'candidates: for source in [
            LinkAcquisition::PublishedEndpoint,
            LinkAcquisition::LocalEndpoint,
        ] {
            for candidate in candidates.iter() {
                for (endpoint, acquisition) in candidate
                    .endpoints
                    .iter()
                    .enumerate()
                    .filter(|(_, acquisition)| **acquisition == source)
                {
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
                            endpoint,
                            candidate.timeout,
                        )
                        .await;
                        if lifetime.check().is_err() {
                            return;
                        }
                        if backoff(&shared, &lifetime, &mut updates).await.is_err() {
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
                            .connect(shared.identity.snapshot(), &candidate.key, endpoint)
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
                                acquisition: acquisition.clone(),
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
                    if backoff(&shared, &lifetime, &mut updates).await.is_err() {
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
        if backoff(&shared, &lifetime, &mut updates).await.is_err() {
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
    }
}

async fn backoff(
    shared: &Shared,
    lifetime: &Scope,
    updates: &mut tokio::sync::watch::Receiver<DataPlaneIdentity>,
) -> Result<(), Error> {
    lifetime
        .run(async {
            if shared.epoch_changed() {
                return Err(Error::Authority);
            }
            tokio::select! { biased;
                _ = updates.changed() => Err(Error::Authority),
                _ = tokio::time::sleep(shared.limits.retry_backoff) => Ok(()),
            }
        })
        .await
}

pub(super) async fn listen(
    mut shared: Arc<Shared>,
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
                        if shared.listener_handoff(fact.socket_id).is_err() {
                            break;
                        }
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
        let cfg = shared.identity.snapshot().configuration();
        let Ok(cfg) = cfg else {
            continue;
        };
        let now = shared.identity.snapshot().now_ms();
        let Ok(now) = now else {
            continue;
        };
        let carriers: Vec<_> = cfg
            .peers
            .keys()
            .filter_map(|peer| {
                let carrier = Carrier {
                    dialer: *peer,
                    listener: cfg.node_id,
                    listener_key: fact.key.clone(),
                    protocol: fact.protocol,
                };
                admit_adjacency(cfg, *peer, &carrier, now)
                    .ok()
                    .map(|_| (*peer, carrier))
            })
            .collect();
        if carriers.is_empty() {
            continue;
        }
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
                    adjacency::accept(&shared.identity, &scope, stream, &carriers).await?;
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
