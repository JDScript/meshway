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

/// Longest pause between two attempts of one failing candidate.
pub(super) const MAX_CANDIDATE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(300);

/// The pause after `failures` consecutive failures: `base · 2^(failures − 1)`,
/// at most [`MAX_CANDIDATE_BACKOFF`].
pub(super) fn backoff(base: std::time::Duration, failures: u32) -> std::time::Duration {
    base.saturating_mul(1 << failures.saturating_sub(1).min(16))
        .min(MAX_CANDIDATE_BACKOFF)
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

/// One dial target: candidate key and endpoint.
type TargetKey = (String, String, meshway_transport::spec::Port);

/// Candidate pacing of one peer worker. Every candidate is tried, whatever
/// its address class; a failed one waits `retry_backoff · 2^(failures − 1)`,
/// at most [`MAX_CANDIDATE_BACKOFF`], before its next attempt. A channel with
/// the peer (selected now, or installed since the last look), a changed
/// candidate set (the peer's endpoint facts) or new netmap content start
/// every candidate over. Addresses only say where to try: identity and
/// admission stay with the handshake.
#[derive(Default)]
pub(super) struct Pacing {
    reference: Option<meshway_model::NetmapRef>,
    targets: std::collections::BTreeSet<TargetKey>,
    own: std::collections::BTreeSet<String>,
    last_link: Option<[u8; 32]>,
    failed: BTreeMap<TargetKey, (u32, tokio::time::Instant)>,
}

impl Pacing {
    fn key(candidate: &Candidate, target: &Target) -> TargetKey {
        (
            candidate.key.clone(),
            target.endpoint.host.clone(),
            target.endpoint.port,
        )
    }

    /// Start over when the content, the candidate set, this Node's own
    /// mappings or the peer's latest channel changed since the last sweep.
    pub(super) fn sweep(
        &mut self,
        reference: meshway_model::NetmapRef,
        targets: std::collections::BTreeSet<TargetKey>,
        own: std::collections::BTreeSet<String>,
        last_link: Option<[u8; 32]>,
    ) {
        if self.reference != Some(reference)
            || self.targets != targets
            || self.own != own
            || self.last_link != last_link
        {
            self.failed.clear();
        }
        self.reference = Some(reference);
        self.targets = targets;
        self.own = own;
        self.last_link = last_link;
    }

    /// A channel with the peer exists: its candidates start over later.
    pub(super) fn connected(&mut self, last_link: Option<[u8; 32]>) {
        self.failed.clear();
        self.last_link = last_link;
    }

    pub(super) fn due(&self, key: &TargetKey, now: tokio::time::Instant) -> bool {
        self.failed.get(key).is_none_or(|(_, at)| *at <= now)
    }

    /// Record a failed attempt; returns its consecutive failures and pause.
    pub(super) fn failed(
        &mut self,
        key: TargetKey,
        base: std::time::Duration,
        now: tokio::time::Instant,
    ) -> (u32, std::time::Duration) {
        let entry = self.failed.entry(key).or_insert((0, now));
        entry.0 = entry.0.saturating_add(1);
        let pause = backoff(base, entry.0);
        entry.1 = now + pause;
        (entry.0, pause)
    }

    /// When the earliest paused candidate of the current set is due again.
    pub(super) fn next_due(&self) -> Option<tokio::time::Instant> {
        self.targets
            .iter()
            .map(|key| self.failed.get(key).map(|(_, at)| *at))
            .min()
            .flatten()
    }
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
            pacing.connected(last_link(&shared, peer));
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
        pacing.sweep(
            plan.reference,
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
            last_link(&shared, peer),
        );
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
    let deadline =
        until.unwrap_or_else(|| tokio::time::Instant::now() + shared.limits.retry_backoff);
    lifetime
        .run(async {
            loop {
                tokio::select! { biased;
                    result = plans.changed() => return result.map_err(|_| Error::Cancelled),
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
mod tests {
    use super::*;
    use std::time::Duration;

    fn reference(digest: u8) -> meshway_model::NetmapRef {
        meshway_model::NetmapRef {
            revision: meshway_model::Revision::MIN,
            serial: meshway_model::Version::MIN,
            digest: [digest; 32].into(),
        }
    }

    fn key(host: &str) -> TargetKey {
        (
            "peer/udp".to_owned(),
            host.to_owned(),
            meshway_transport::spec::Port::new(35000).unwrap(),
        )
    }

    #[test]
    fn a_failing_candidate_doubles_its_pause_up_to_five_minutes() {
        let now = tokio::time::Instant::now();
        let (private, public) = (key("192.168.139.66"), key("198.51.100.7"));
        let mut pacing = Pacing::default();
        pacing.sweep(
            reference(1),
            [private.clone(), public.clone()].into(),
            Default::default(),
            None,
        );
        let base = Duration::from_secs(1);
        let pauses: Vec<_> = (0..11)
            .map(|_| pacing.failed(private.clone(), base, now).1.as_secs())
            .collect();
        assert_eq!(pauses, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        assert!(!pacing.due(&private, now + Duration::from_secs(299)));
        assert!(pacing.due(&private, now + MAX_CANDIDATE_BACKOFF));
        // Every other candidate, whatever its address class, keeps its own pace.
        assert!(pacing.due(&public, now));
        assert_eq!(pacing.next_due(), None, "an untried candidate is due now");
        pacing.failed(public.clone(), base, now);
        assert_eq!(pacing.next_due(), Some(now + base));
    }

    #[test]
    fn content_facts_or_a_channel_start_every_candidate_over() {
        let now = tokio::time::Instant::now();
        let base = Duration::from_secs(1);
        let (old, new) = (key("192.168.139.66"), key("203.0.113.9"));
        let failed = |pacing: &mut Pacing| {
            for _ in 0..5 {
                pacing.failed(old.clone(), base, now);
            }
            assert!(!pacing.due(&old, now + base));
        };
        let mut pacing = Pacing::default();
        pacing.sweep(reference(1), [old.clone()].into(), Default::default(), None);
        failed(&mut pacing);
        // The same content, candidates and channel keep the pause.
        pacing.sweep(reference(1), [old.clone()].into(), Default::default(), None);
        assert!(!pacing.due(&old, now + base));
        // The peer's endpoint facts changed.
        pacing.sweep(
            reference(1),
            [old.clone(), new.clone()].into(),
            Default::default(),
            None,
        );
        assert!(pacing.due(&old, now));
        failed(&mut pacing);
        // This Node's own mapping changed (another egress address).
        pacing.sweep(
            reference(1),
            [old.clone(), new.clone()].into(),
            ["203.0.113.9:40001".to_owned()].into(),
            None,
        );
        assert!(pacing.due(&old, now));
        failed(&mut pacing);
        // New netmap content.
        pacing.sweep(
            reference(2),
            [old.clone(), new.clone()].into(),
            Default::default(),
            None,
        );
        assert!(pacing.due(&old, now));
        failed(&mut pacing);
        // A channel was installed (and may have ended) since the last sweep.
        pacing.sweep(
            reference(2),
            [old.clone(), new.clone()].into(),
            Default::default(),
            Some([7; 32]),
        );
        assert!(pacing.due(&old, now));
        failed(&mut pacing);
        // A selected channel exists now.
        pacing.connected(Some([7; 32]));
        assert!(pacing.due(&old, now));
    }
}
