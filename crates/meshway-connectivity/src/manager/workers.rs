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

/// One peer's acquisition owner for the Manager's lifetime. Every sweep
/// reads the current plan; Manager's apply stops this owner when the peer is
/// no longer a dial peer.
pub(super) async fn dial(shared: Arc<Shared>, factory: Arc<dyn SocketFactory>, peer: NodeId) {
    let lifetime = shared.scope(None, false);
    let mut plans = shared.plans.subscribe();
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
            if backoff(&shared, &lifetime, peer).await.is_err() {
                return;
            }
            continue;
        }
        plans.borrow_and_update();
        let plan = shared.plan();
        let Ok(candidates) = candidates(&shared, &plan, peer) else {
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
                            plan.clone(),
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
                        if backoff(&shared, &lifetime, peer).await.is_err() {
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
                        let (stream, id) = adjacency::dial(
                            &shared.keys,
                            &plan,
                            &scope,
                            stream,
                            &candidate.carrier,
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
                        let _ = connected(
                            shared.clone(),
                            stream,
                            fact,
                            plan.clone(),
                            opening,
                            &mut was_selected,
                        )
                        .await;
                    }
                    drop(capacity);
                    if lifetime.check().is_err() {
                        return;
                    }
                    // Every failure/rejected duplicate is paced; no immediate
                    // failed-dial loop, including single-candidate contents.
                    if backoff(&shared, &lifetime, peer).await.is_err() {
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
        if backoff(&shared, &lifetime, peer).await.is_err() {
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

/// Paced retry. New peer facts or new content end the wait early (a changed
/// candidate set may be dialable now); interest alone never shortens the pace.
async fn backoff(shared: &Shared, lifetime: &Scope, peer: NodeId) -> Result<(), Error> {
    let facts = shared.facts.get(peer);
    let wake = shared.interests.waker(peer);
    let mut plans = shared.plans.subscribe();
    let deadline = tokio::time::Instant::now() + shared.limits.retry_backoff;
    lifetime
        .run(async {
            loop {
                tokio::select! { biased;
                    result = plans.changed() => return result.map_err(|_| Error::Cancelled),
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
                let (stream, peer, carrier, id) =
                    adjacency::accept(&shared.keys, &plan, &scope, stream, carriers).await?;
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
                let _ = connected(shared, stream, fact, plan, opening, &mut was_selected).await;
            }
        });
    }
    drop(listener);
    shared.stop.cancel();
    while children.join_next().await.is_some() {}
}
