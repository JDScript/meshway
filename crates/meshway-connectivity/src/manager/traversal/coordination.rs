use super::*;

/// Native UDP carriers with `peer` the plan admits, in either direction.
pub(super) fn udp_carrier(plan: &Plan, carrier: &Carrier) -> bool {
    let peer = if carrier.dialer == plan.node {
        carrier.listener
    } else {
        carrier.dialer
    };
    carrier.protocol == TransportProtocol::NativeUdp && plan.admit(peer, carrier).is_ok()
}

/// Coordination pacing of one content. An attempt that ended without a
/// channel pauses this Node's own preparation toward that peer, and the
/// queries made for it, for [`workers::backoff`] (1 s doubling, at most one
/// minute). A channel with the peer, a change of its endpoint facts, of this
/// edge in the content ([`Plan::edge`]) or of this Node's own mappings ends
/// the pause; other content changes keep it. The pause never delays taking
/// part in an attempt the peer started: a live request it sent is queried
/// and accepted at once (see [`needs_acquisition`]), without preparing.
/// Peers that leave the content leave the table; it holds at most
/// [`MAX_COORDINATED`] peers.
#[derive(Default)]
pub(super) struct Coordination(BTreeMap<NodeId, Paused>);

/// Peers whose coordination pause one Manager keeps.
const MAX_COORDINATED: usize = 1024;

struct Paused {
    failures: u32,
    until: Instant,
    edge: crate::plan::Edge,
    link: Option<[u8; 32]>,
    facts: std::collections::BTreeSet<String>,
    own: std::collections::BTreeSet<String>,
}

/// The peer's listener endpoint addresses, without their freshness.
fn endpoint_facts(shared: &Shared, peer: NodeId) -> std::collections::BTreeSet<String> {
    let Some(facts) = shared.facts.get(peer) else {
        return Default::default();
    };
    facts
        .listeners
        .iter()
        .flat_map(|(key, listener)| {
            listener
                .local_endpoints
                .iter()
                .map(move |endpoint| format!("{key} {}", endpoint.address))
                .chain(
                    listener
                        .stun_candidates
                        .iter()
                        .map(move |candidate| format!("{key} {}", candidate.address)),
                )
        })
        .collect()
}

fn last_link(shared: &Shared, peer: NodeId) -> Option<[u8; 32]> {
    shared
        .links
        .lock()
        .ok()
        .and_then(|links| links.last.get(&peer).copied())
}

impl Coordination {
    /// New content: forget peers it no longer holds.
    pub(super) fn retain(&mut self, plan: &crate::plan::Plan) {
        self.0.retain(|peer, _| plan.has_peer(*peer));
    }

    /// An attempt with `peer` ended; without a channel it is one failure.
    pub(super) fn ended(&mut self, shared: &Shared, peer: NodeId) {
        if shared
            .links
            .lock()
            .is_ok_and(|links| links.contains_key(&peer))
        {
            self.0.remove(&peer);
            return;
        }
        let link = last_link(shared, peer);
        let facts = endpoint_facts(shared, peer);
        let own = workers::own_mappings(shared);
        let edge = shared.plan().edge(peer);
        if self.0.len() >= MAX_COORDINATED && !self.0.contains_key(&peer) {
            // Bounded: the peer whose pause ended longest ago gives way;
            // while every one is still paused the new failure is not kept.
            let now = Instant::now();
            let Some(oldest) = self
                .0
                .iter()
                .filter(|(_, paused)| paused.until <= now)
                .min_by_key(|(_, paused)| paused.until)
                .map(|(peer, _)| *peer)
            else {
                return;
            };
            self.0.remove(&oldest);
        }
        let entry = self.0.entry(peer).or_insert(Paused {
            failures: 0,
            until: Instant::now(),
            edge: edge.clone(),
            link,
            facts: facts.clone(),
            own: own.clone(),
        });
        if entry.edge != edge || entry.link != link || entry.facts != facts || entry.own != own {
            entry.failures = 0;
        }
        entry.failures = entry.failures.saturating_add(1);
        entry.until = Instant::now() + workers::backoff(Duration::from_secs(1), entry.failures);
        entry.edge = edge;
        entry.link = link;
        entry.facts = facts;
        entry.own = own;
        tracing::debug!(%peer, failures = entry.failures,
            retry_ms = entry.until.saturating_duration_since(Instant::now()).as_millis() as u64,
            "Traversal attempt ended without a channel");
    }

    pub(super) fn paused(&self, shared: &Shared, peer: NodeId) -> bool {
        self.0.get(&peer).is_some_and(|paused| {
            Instant::now() < paused.until
                && last_link(shared, peer) == paused.link
                && shared.plan().edge(peer) == paused.edge
                && endpoint_facts(shared, peer) == paused.facts
                && workers::own_mappings(shared) == paused.own
        })
    }
}

pub(super) fn needs_acquisition(
    shared: &Shared,
    pending: &BTreeMap<UdpTraversalId, Pending>,
    coordination: &Coordination,
) -> bool {
    let plan = &*shared.plan();
    plan.peers().any(|peer| {
        // The selected owner retires unhealthy channels. A selected channel is
        // not a vacant physical slot: prepare() must wait for that owner too.
        if shared
            .links
            .lock()
            .is_ok_and(|links| links.contains_key(&peer))
            || !shared.wanted(peer)
            || (coordination.paused(shared, peer) && !shared.facts.asked(peer, wall_ms()))
        {
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
            !needs_acquisition(&shared, &BTreeMap::new(), &Coordination::default())
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
            assert!(needs_acquisition(
                &shared,
                &BTreeMap::new(),
                &Coordination::default()
            ));
        }
        fixture.close().await;
    }

    /// The pause paces only what this Node starts. A peer that restarted
    /// prepares toward it at once; the paused side must still query and
    /// accept that attempt immediately, or both ends stay out of step.
    #[tokio::test]
    async fn a_paused_peer_that_asks_is_accepted_at_once_without_preparing() {
        let fixture = Netmaps::start(false, false).await;
        let ids = fixture.ids();
        let (shared, _) = shared(&fixture.desired[0], &fixture.keys[0]);
        let peer = ids[1];
        shared.interests.touch(peer);
        let mut coordination = Coordination::default();
        assert!(needs_acquisition(&shared, &BTreeMap::new(), &coordination));
        for _ in 0..9 {
            coordination.ended(&shared, peer);
        }
        assert!(coordination.paused(&shared, peer));
        assert!(
            !needs_acquisition(&shared, &BTreeMap::new(), &coordination),
            "paused: no own query"
        );
        let (publish, source) = tokio::sync::watch::channel(None);
        shared.facts.attach(source);
        publish.send_replace(Some(meshway_control::KnownPeers {
            netmap: shared.plan().reference,
            peers: Arc::default(),
            traversal_requests: Arc::new(vec![meshway_protocol::UdpTraversalRequest {
                attempt_id: UdpTraversalId::new(uuid::Uuid::from_u128(7)).unwrap(),
                peer_id: peer,
                carrier: Carrier {
                    dialer: peer,
                    listener: ids[0],
                    listener_key: "udp".into(),
                    protocol: TransportProtocol::NativeUdp,
                },
                expires_at_ms: wall_ms() + 9_000,
            }]),
        }));
        assert!(
            needs_acquisition(&shared, &BTreeMap::new(), &coordination),
            "the peer's request is queried for at once"
        );
        assert!(
            coordination.paused(&shared, peer),
            "this Node's own preparation stays paused"
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn an_attempt_without_a_channel_pauses_coordination_toward_that_peer() {
        let fixture = Netmaps::start(false, false).await;
        let ids = fixture.ids();
        let (shared, _) = shared(&fixture.desired[0], &fixture.keys[0]);
        let peer = ids[1];
        let mut coordination = Coordination::default();
        assert!(!coordination.paused(&shared, peer));
        let pauses: Vec<_> = (0..10)
            .map(|_| {
                coordination.ended(&shared, peer);
                let until = coordination.0[&peer].until;
                (until - Instant::now()).as_secs_f64().round() as u64
            })
            .collect();
        assert_eq!(pauses, [1, 2, 4, 8, 16, 32, 60, 60, 60, 60]);
        assert!(coordination.paused(&shared, peer));
        assert!(!coordination.paused(&shared, ids[2]), "per peer");
        // The peer's endpoint facts changed since the failure: try again now,
        // and the next failure starts the sequence over.
        coordination.0.get_mut(&peer).unwrap().facts = ["udp 192.0.2.1:1".to_owned()].into();
        assert!(!coordination.paused(&shared, peer));
        coordination.ended(&shared, peer);
        assert_eq!(coordination.0[&peer].failures, 1);
        // So does a new mapping of this Node's own.
        coordination.0.get_mut(&peer).unwrap().own = ["203.0.113.9:40001".to_owned()].into();
        assert!(!coordination.paused(&shared, peer));
        coordination.ended(&shared, peer);
        // So does a change of this edge in the content.
        coordination.0.get_mut(&peer).unwrap().edge = crate::plan::Edge::default();
        assert!(!coordination.paused(&shared, peer));
        coordination.ended(&shared, peer);
        assert_eq!(coordination.0[&peer].failures, 1);
        assert!(coordination.paused(&shared, peer));
        // So does a channel installed (and maybe ended) since.
        coordination.0.get_mut(&peer).unwrap().link = Some([9; 32]);
        assert!(!coordination.paused(&shared, peer));
        fixture.close().await;
    }

    /// New content drops the pauses of peers it no longer holds, and the
    /// table never exceeds its bound: a new peer displaces the one whose
    /// pause ended longest ago, never one still paused.
    #[tokio::test]
    async fn coordination_pauses_are_bounded_and_follow_the_content() {
        let fixture = Netmaps::start(false, false).await;
        let ids = fixture.ids();
        let (shared, _) = shared(&fixture.desired[0], &fixture.keys[0]);
        let unknown = |i: usize| -> NodeId {
            format!("00000000-0000-4000-8000-{:012}", 9000 + i)
                .parse()
                .unwrap()
        };
        let paused = |until| Paused {
            failures: 1,
            until,
            edge: crate::plan::Edge::default(),
            link: None,
            facts: Default::default(),
            own: Default::default(),
        };
        let mut coordination = Coordination::default();
        coordination.ended(&shared, ids[1]);
        // A peer the content no longer holds (removed, or out of view).
        coordination.0.insert(unknown(0), paused(Instant::now()));
        assert!(!shared.plan().has_peer(unknown(0)));
        coordination.retain(&shared.plan());
        assert_eq!(coordination.0.keys().copied().collect::<Vec<_>>(), [ids[1]]);

        let mut full = Coordination::default();
        let paused = |until| Paused {
            failures: 1,
            until,
            edge: crate::plan::Edge::default(),
            link: None,
            facts: Default::default(),
            own: Default::default(),
        };
        let now = Instant::now();
        for i in 0..MAX_COORDINATED {
            full.0
                .insert(unknown(i), paused(now + Duration::from_secs(30)));
        }
        full.ended(&shared, ids[1]);
        assert_eq!(full.0.len(), MAX_COORDINATED);
        assert!(!full.0.contains_key(&ids[1]), "every entry still paused");
        full.0
            .insert(unknown(0), paused(now - Duration::from_secs(5)));
        full.0
            .insert(unknown(1), paused(now - Duration::from_secs(1)));
        full.ended(&shared, ids[1]);
        assert_eq!(full.0.len(), MAX_COORDINATED);
        assert!(full.0.contains_key(&ids[1]));
        assert!(
            !full.0.contains_key(&unknown(0)),
            "the oldest ended pause gave way"
        );
        assert!(full.0.contains_key(&unknown(1)));
        fixture.close().await;
    }
}
