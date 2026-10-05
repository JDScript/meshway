use super::*;
use crate::ListenerSocket;

pub(super) struct Candidate {
    key: String,
    carrier: Carrier,
    endpoints: usize,
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
    let now = identity.snapshot().now_ms().map_err(|_| Error::Authority)?;
    let mut plans: BTreeMap<NodeId, Vec<Candidate>> = BTreeMap::new();
    let mut endpoints = 0usize;
    for (key, candidate) in &cfg.dialers {
        let carrier = Carrier {
            dialer: cfg.node_id,
            listener: candidate.peer,
            listener_key: candidate.listener_key.clone(),
            protocol: candidate.parameters.transport(),
        };
        if admit_adjacency(cfg, candidate.peer, &carrier, now).is_err() {
            continue;
        }
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
        endpoints = endpoints
            .checked_add(candidate.endpoints.len())
            .ok_or(Error::Capacity)?;
        if endpoints > limits.candidates {
            return Err(Error::Capacity);
        }
        if candidate.endpoints.is_empty() {
            continue;
        }
        plans.entry(candidate.peer).or_default().push(Candidate {
            key: key.clone(),
            carrier,
            endpoints: candidate.endpoints.len(),
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

pub(super) async fn dial(
    shared: Arc<Shared>,
    factory: Arc<dyn SocketFactory>,
    peer: NodeId,
    candidates: Vec<Candidate>,
) {
    let lifetime = shared.scope(None, false);
    loop {
        if lifetime.check().is_err() {
            return;
        }
        let active = shared.links.lock().is_ok_and(|links| {
            links
                .get(&peer)
                .is_some_and(|link| link.scope.check().is_ok())
        });
        if active {
            if lifetime
                .run(async {
                    tokio::time::sleep(shared.limits.retry_backoff).await;
                    Ok(())
                })
                .await
                .is_err()
            {
                return;
            }
            continue;
        }
        'candidates: for candidate in &candidates {
            for endpoint in 0..candidate.endpoints {
                if shared.links.lock().is_ok_and(|links| {
                    links
                        .get(&peer)
                        .is_some_and(|link| link.scope.check().is_ok())
                }) {
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
                    if lifetime
                        .run(async {
                            tokio::time::sleep(shared.limits.retry_backoff).await;
                            Ok(())
                        })
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
                    let socket = factory
                        .connect(shared.identity.snapshot(), &candidate.key, endpoint)
                        .await?;
                    let raw = Box::new(scope::Stream {
                        inner: socket.stream,
                        scope: shared.scope(Some((peer, candidate.carrier.clone())), false),
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
                        },
                    ))
                };
                if let Ok((stream, fact)) = scope.run(attempt).await {
                    let _ = connected(shared.clone(), stream, fact).await;
                }
                drop(capacity);
                if lifetime.check().is_err() {
                    return;
                }
                // Every failure/rejected duplicate is paced; no immediate
                // failed-dial loop, including single-candidate publications.
                if lifetime
                    .run(async {
                        tokio::time::sleep(shared.limits.retry_backoff).await;
                        Ok(())
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
        if lifetime
            .run(async {
                tokio::time::sleep(shared.limits.retry_backoff).await;
                Ok(())
            })
            .await
            .is_err()
        {
            return;
        }
    }
}

pub(super) async fn listen(
    shared: Arc<Shared>,
    fact: ListenerFact,
    listener: Box<dyn ListenerSocket>,
    acceptor: Arc<provider::Acceptor>,
) {
    let lifetime = shared.scope(None, false);
    let mut children = JoinSet::new();
    let operation = async {
        loop {
            let socket = tokio::select! { biased;
                result = children.join_next(), if !children.is_empty() => {
                    if result.is_some_and(|result| result.is_err()) {
                        shared.stop.cancel();
                        return Err(Error::Worker);
                    }
                    continue;
                },
                socket = listener.accept() => match socket {
                    Ok(socket) => socket,
                    Err(_) => { shared.stop.cancel(); return Err(Error::Underlay); },
                },
            };
            let Ok(capacity) = shared.capacity.clone().try_acquire_owned() else {
                continue;
            };
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
                    let raw = Box::new(scope::Stream {
                        inner: socket.stream,
                        scope: shared.scope(None, false),
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
                        },
                    ))
                };
                if let Ok((stream, fact)) = scope.run(attempt).await {
                    let _ = connected(shared, stream, fact).await;
                }
            });
        }
    };
    let _: Result<(), Error> = lifetime.run(operation).await;
    drop(listener);
    while children.join_next().await.is_some() {}
}
