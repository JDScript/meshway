//! Desired netmap content: on-demand adjacency, idle close, resident edges,
//! peer facts without reconfiguration and per-peer serial differentials, over
//! real loopback sockets and Control-verified netmaps.
use super::*;
use meshway_configuration::{TopologyAction, TopologyRule};
use meshway_control::KnownPeers;
use meshway_model::state::{ListenerPhase, LocalEndpoint, LocalEndpointSource, SocketNetwork};
use meshway_protocol::{PeerFacts, PeerListenerFacts};
use meshway_transport::spec::NativeUdpDialer;
use support::{Netmaps, eventually, links};

pub(in crate::manager) mod support;

fn limits(idle: Duration) -> Limits {
    Limits {
        idle,
        ..Default::default()
    }
}

async fn manager(
    fixture: &Netmaps,
    index: usize,
    factory: &Arc<Factory>,
    limits: Limits,
) -> Manager {
    Manager::start(
        &fixture.desired[index],
        fixture.keys[index].clone(),
        factory.clone(),
        limits,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn netmap_adjacency_is_acquired_only_on_interest_for_admitted_peers() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    // Every dialable peer has a worker, but none acquires without interest.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(links(&a).is_empty() && links(&b).is_empty());
    assert_eq!(factory.acquisitions.load(Ordering::SeqCst), 0);
    // Interest in a non-visible Node is ignored.
    let unknown: NodeId = "00000000-0000-4000-8000-0000000000aa".parse().unwrap();
    a.interest().interest(unknown);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(factory.acquisitions.load(Ordering::SeqCst), 0);
    a.interest().interest(ids[1]);
    eventually("interest acquires a->b", || links(&a).contains_key(&ids[1])).await;
    let link = links(&a)[&ids[1]].clone();
    assert_eq!(link.carrier.protocol, TransportProtocol::NativeUdp);
    assert_eq!(link.acquisition, LinkAcquisition::PublishedEndpoint);
    assert!(!links(&a).contains_key(&ids[2]), "c was not requested");
    eventually("b accepts the inbound channel", || {
        links(&b).contains_key(&ids[0])
    })
    .await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn idle_adjacency_closes_without_interest_or_frames_and_reacquires_on_interest() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let idle = Duration::from_millis(1500);
    let mut a = manager(&fixture, 0, &factory, limits(idle)).await;
    let mut b = manager(&fixture, 1, &factory, limits(idle)).await;
    a.interest().interest(ids[1]);
    eventually("interest acquires a->b", || links(&a).contains_key(&ids[1])).await;
    let first = links(&a)[&ids[1]].id;
    // Repeated interest keeps it; without interest it closes on both sides.
    for _ in 0..4 {
        tokio::time::sleep(idle / 3).await;
        a.interest().interest(ids[1]);
    }
    assert_eq!(links(&a)[&ids[1]].id, first);
    eventually("idle channel closes", || links(&a).is_empty()).await;
    eventually("idle inbound channel closes", || links(&b).is_empty()).await;
    let acquisitions = factory.acquisitions.load(Ordering::SeqCst);
    tokio::time::sleep(idle).await;
    assert_eq!(
        factory.acquisitions.load(Ordering::SeqCst),
        acquisitions,
        "no reacquisition without interest"
    );
    a.interest().interest(ids[1]);
    eventually("interest reacquires", || {
        links(&a).get(&ids[1]).is_some_and(|link| link.id != first)
    })
    .await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn keepalive_edge_is_resident_without_interest() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (a_id, b_id) = (ids[0], ids[1]);
    fixture
        .publish(|cfg| {
            cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: format!("node:{a_id}"),
                b: format!("node:{b_id}"),
                transports: meshway_configuration::Input::Omitted,
                keepalive: true,
            })
        })
        .await;
    let factory = fixture.factory();
    let idle = Duration::from_millis(500);
    let mut a = manager(&fixture, 0, &factory, limits(idle)).await;
    let mut b = manager(&fixture, 1, &factory, limits(idle)).await;
    let mut c = manager(&fixture, 2, &factory, limits(idle)).await;
    // Both ends dial a resident edge; each keeps the channel dialed by the
    // lower Node ID (`rank`), so an earlier inbound channel may be replaced
    // once. Wait for that converged channel, held by both ends.
    let preferred = a_id.min(b_id);
    eventually("keepalive edge converges on one channel", || {
        let (a, b) = (links(&a), links(&b));
        a.get(&b_id).is_some_and(|link| {
            link.carrier.dialer == preferred && b.get(&a_id).is_some_and(|l| l.id == link.id)
        })
    })
    .await;
    let id = links(&a)[&ids[1]].id;
    tokio::time::sleep(idle * 4).await;
    assert_eq!(links(&a)[&ids[1]].id, id, "keepalive is never idle-closed");
    assert!(!links(&a).contains_key(&ids[2]), "c is on demand");
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn wildcard_keepalive_keeps_every_edge_of_three_nodes_resident() {
    // `* ↔ *` with keepalive: every edge is dialable both ways and resident,
    // although no rule names a Node.
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    fixture
        .publish(|cfg| {
            cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: "*".to_owned(),
                b: "*".to_owned(),
                transports: meshway_configuration::Input::Omitted,
                keepalive: true,
            })
        })
        .await;
    let factory = fixture.factory();
    let idle = Duration::from_millis(500);
    let managers = [
        manager(&fixture, 0, &factory, limits(idle)).await,
        manager(&fixture, 1, &factory, limits(idle)).await,
        manager(&fixture, 2, &factory, limits(idle)).await,
    ];
    let pairs = [(0, 1), (0, 2), (1, 2)];
    let converged = || {
        pairs.iter().all(|&(x, y)| {
            let (from, to) = (links(&managers[x]), links(&managers[y]));
            from.get(&ids[y])
                .is_some_and(|link| to.get(&ids[x]).is_some_and(|back| back.id == link.id))
        })
    };
    eventually("every wildcard keepalive edge converges", converged).await;
    let before: Vec<_> = pairs
        .iter()
        .map(|&(x, y)| links(&managers[x])[&ids[y]].id)
        .collect();
    tokio::time::sleep(idle * 4).await;
    let after: Vec<_> = pairs
        .iter()
        .map(|&(x, y)| links(&managers[x])[&ids[y]].id)
        .collect();
    assert_eq!(after, before, "no resident edge is idle-closed");
    for mut manager in managers {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}

#[tokio::test]
async fn failing_candidates_back_off_across_unrelated_content_and_start_over_on_their_edge() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (a_id, b_id, c_id) = (ids[0], ids[1], ids[2]);
    fixture
        .publish(|cfg| {
            cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: format!("node:{a_id}"),
                b: format!("node:{b_id}"),
                transports: meshway_configuration::Input::Omitted,
                keepalive: true,
            })
        })
        .await;
    let factory = fixture.factory();
    factory.blocked_dialers.lock().unwrap().extend([a_id, b_id]);
    let limits = Limits {
        retry_backoff: Duration::from_millis(100),
        ..limits(Duration::from_millis(500))
    };
    let start = tokio::time::Instant::now();
    let mut a = manager(&fixture, 0, &factory, limits).await;
    let mut b = manager(&fixture, 1, &factory, limits).await;
    // Each end's one candidate fails at once: attempts at about 0, 0.1, 0.3,
    // 0.7, 1.5, 3.1, 6.3 and 12.7 s, then 25.5 s (a fixed pace would be ~130).
    tokio::time::sleep_until(start + Duration::from_millis(3600)).await;
    let early = factory.acquisitions.load(Ordering::SeqCst);
    assert!((8..=14).contains(&early), "{early} attempts in 3.6 s");
    tokio::time::sleep_until(start + Duration::from_millis(13_500)).await;
    let paused = factory.acquisitions.load(Ordering::SeqCst);
    assert!(
        paused - early <= 6,
        "{} attempts until 13.5 s",
        paused - early
    );
    // Content that does not concern the a–b edge (an Access rule, another
    // Node's remark) keeps the pause: nothing is tried before 25.5 s.
    factory.blocked_dialers.lock().unwrap().clear();
    fixture.reserial(0).await;
    let changed = fixture
        .publish(|cfg| cfg.nodes.get_mut(&c_id).unwrap().remark = "unrelated".into())
        .await;
    assert!(changed[0] && changed[1]);
    apply_all(&fixture, &mut [&mut a, &mut b]).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        factory.acquisitions.load(Ordering::SeqCst),
        paused,
        "unrelated content restarted paused candidates"
    );
    assert!(links(&a).is_empty() && links(&b).is_empty());
    // A change of the edge's own dialers starts them over at once, long
    // before their next attempt.
    let changed = fixture
        .publish(|cfg| {
            cfg.defaults.dialers.native_udp = Some(NativeUdpDialer {
                connect_timeout: "3s".parse().unwrap(),
                ..Default::default()
            })
        })
        .await;
    assert!(changed[0] && changed[1]);
    let resumed = tokio::time::Instant::now();
    apply_all(&fixture, &mut [&mut a, &mut b]).await;
    eventually("the edge connects after its own change", || {
        let (a, b) = (links(&a), links(&b));
        a.get(&b_id)
            .is_some_and(|link| b.get(&a_id).is_some_and(|back| back.id == link.id))
    })
    .await;
    assert!(resumed.elapsed() < Duration::from_secs(4));
    assert!(start.elapsed() < Duration::from_millis(25_500));
    // A selected channel stops every probe of the peer's candidates.
    let connected = factory.acquisitions.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(factory.acquisitions.load(Ordering::SeqCst), connected);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn one_way_edge_is_resident_at_both_ends_without_interest() {
    // Only b listens (TCP): a and c can dial b, b can dial neither.
    let mut fixture = Netmaps::start(true, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let idle = Duration::from_millis(500);
    let mut a = manager(&fixture, 0, &factory, limits(idle)).await;
    let mut b = manager(&fixture, 1, &factory, limits(idle)).await;
    let mut c = manager(&fixture, 2, &factory, limits(idle)).await;
    for (from, to) in [(&a, ids[1]), (&c, ids[1]), (&b, ids[0]), (&b, ids[2])] {
        eventually("one-way edge acquires without interest", || {
            links(from).contains_key(&to)
        })
        .await;
    }
    let before = (links(&a)[&ids[1]].id, links(&b)[&ids[0]].id);
    tokio::time::sleep(idle * 4).await;
    assert_eq!(
        (links(&a)[&ids[1]].id, links(&b)[&ids[0]].id),
        before,
        "neither the dialer nor the listener-only end idle-closes it"
    );
    assert!(
        !links(&a).contains_key(&ids[2]) && !links(&c).contains_key(&ids[0]),
        "a and c share no carrier"
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

fn facts(peer: NodeId, address: SocketAddr) -> PeerFacts {
    let now = chrono::Utc::now();
    PeerFacts {
        node_id: peer,
        listeners: BTreeMap::from([(
            "udp".to_owned(),
            PeerListenerFacts {
                phase: ListenerPhase::Listening,
                network: SocketNetwork::Udp,
                local_endpoints: vec![LocalEndpoint {
                    address,
                    source: LocalEndpointSource::Local,
                }],
                stun_candidates: vec![],
            },
        )]),
        exit: None,
        connected: Vec::new(),
        observed_at: now,
        expires_at: now + chrono::Duration::minutes(5),
    }
}

/// Control's publication of its last known facts.
fn known(netmap: meshway_model::NetmapRef, peers: Vec<PeerFacts>) -> Option<KnownPeers> {
    Some(KnownPeers {
        netmap,
        peers: Arc::new(
            peers
                .into_iter()
                .map(|facts| (facts.node_id, facts))
                .collect(),
        ),
        traversal_requests: Default::default(),
    })
}

#[tokio::test]
async fn peer_facts_add_dial_endpoints_without_reconfiguring_or_touching_other_peers() {
    // b's listener has no configured endpoint: only facts can make it dialable.
    let mut fixture = Netmaps::start(false, false).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let Listener::NativeUdp(listener) = fixture.cfg.nodes[&ids[1]].listeners["udp"].declaration()
    else {
        unreachable!()
    };
    let b_address = SocketAddr::new(listener.listen, listener.listen_port.get());
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    let mut c = manager(&fixture, 2, &factory, Default::default()).await;
    assert!(
        a.shared.plan().dial_peers().contains(&ids[1]),
        "an endpoint-less netmap candidate is still dialable through facts"
    );
    a.interest().interest(ids[1]);
    a.interest().interest(ids[2]);
    eventually("configured c is acquired", || {
        links(&a).contains_key(&ids[2])
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!links(&a).contains_key(&ids[1]), "no endpoint for b yet");
    let c_link = links(&a)[&ids[2]].clone();
    let plan = a.shared.plan();
    let netmap = fixture.desired[0].reference();
    let (publish, source) = tokio::sync::watch::channel(None);
    a.attach_facts(source);
    publish.send_replace(known(netmap, vec![facts(ids[1], b_address)]));
    eventually("fact endpoint is dialed", || {
        links(&a).contains_key(&ids[1])
    })
    .await;
    let b_link = links(&a)[&ids[1]].clone();
    assert_eq!(b_link.acquisition, LinkAcquisition::LocalEndpoint);
    assert_eq!(b_link.remote, b_address);
    assert!(
        Arc::ptr_eq(&a.shared.plan(), &plan),
        "facts never reconfigure the Manager"
    );
    assert_eq!(links(&a)[&ids[2]], c_link, "other peers are untouched");
    // Removing the fact keeps the healthy channel; it only shrinks candidates.
    publish.send_replace(known(netmap, vec![]));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(links(&a)[&ids[1]].id, b_link.id);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn stale_peer_facts_still_redial_after_idle_close() {
    // Server unreachable: b's only endpoints are facts past their freshness.
    let mut fixture = Netmaps::start(false, false).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let Listener::NativeUdp(listener) = fixture.cfg.nodes[&ids[1]].listeners["udp"].declaration()
    else {
        unreachable!()
    };
    let b_address = SocketAddr::new(listener.listen, listener.listen_port.get());
    let idle = Duration::from_millis(1500);
    let mut a = manager(&fixture, 0, &factory, limits(idle)).await;
    let mut b = manager(&fixture, 1, &factory, limits(idle)).await;
    let mut stale = facts(ids[1], b_address);
    stale.observed_at -= chrono::Duration::minutes(10);
    stale.expires_at = stale.observed_at + chrono::Duration::seconds(60);
    let netmap = fixture.desired[0].reference();
    let (publish, source) = tokio::sync::watch::channel(None);
    a.attach_facts(source);
    publish.send_replace(known(netmap, vec![stale]));
    a.interest().interest(ids[1]);
    eventually("stale fact endpoint is dialed", || {
        links(&a).contains_key(&ids[1])
    })
    .await;
    let first = links(&a)[&ids[1]].id;
    eventually("idle adjacency closes", || links(&a).is_empty()).await;
    eventually("idle inbound adjacency closes", || links(&b).is_empty()).await;
    // Renewed interest, as new packets toward b would express it.
    eventually("the same stale endpoint is dialed again", || {
        a.interest().interest(ids[1]);
        links(&a).get(&ids[1]).is_some_and(|link| link.id != first)
    })
    .await;
    assert_eq!(links(&a)[&ids[1]].remote, b_address);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

async fn apply_all(fixture: &Netmaps, managers: &mut [&mut Manager]) {
    for (index, manager) in managers.iter_mut().enumerate() {
        tokio::time::timeout(
            Duration::from_secs(2),
            manager.apply(&fixture.desired[index]),
        )
        .await
        .unwrap()
        .unwrap();
    }
}

#[tokio::test]
async fn serial_change_replaces_only_peers_whose_carriers_changed() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    let mut c = manager(&fixture, 2, &factory, Default::default()).await;
    a.interest().interest(ids[1]);
    a.interest().interest(ids[2]);
    eventually("a links b and c", || links(&a).len() == 2).await;
    let before = links(&a);
    let acquisitions = factory.acquisitions.load(Ordering::SeqCst);
    let plan = a.shared.plan();

    // Same content: a lease renewal is a no-op for the Manager.
    assert_eq!(fixture.publish(|_| {}).await, vec![false; 3]);
    apply_all(&fixture, &mut [&mut a, &mut b, &mut c]).await;
    assert!(Arc::ptr_eq(&a.shared.plan(), &plan));
    assert_eq!(links(&a), before);

    // Block a<->c: only that carrier set changes for a.
    let (a_id, c_id) = (ids[0], ids[2]);
    let changed = fixture
        .publish(|cfg| {
            cfg.topology.push(TopologyRule {
                remark: String::new(),
                action: TopologyAction::Block,
                a: format!("node:{a_id}"),
                b: format!("node:{c_id}"),
                transports: meshway_configuration::Input::Omitted,
                keepalive: false,
            })
        })
        .await;
    assert!(changed[0] && changed[2]);
    apply_all(&fixture, &mut [&mut a, &mut b, &mut c]).await;
    assert_eq!(a.shared.plan().reference, fixture.desired[0].reference());
    let after = links(&a);
    assert_eq!(after.get(&ids[1]), before.get(&ids[1]), "a-b channel kept");
    assert!(!after.contains_key(&ids[2]), "a-c no longer admitted");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        factory.acquisitions.load(Ordering::SeqCst),
        acquisitions,
        "no reacquisition: b kept and c not admitted despite interest"
    );
    assert_eq!(links(&a).get(&ids[1]), before.get(&ids[1]));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn native_tcp_stream_survives_an_unrelated_serial_change() {
    let mut fixture = Netmaps::start(true, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    a.interest().interest(ids[1]);
    eventually("a dials b over TCP", || links(&a).contains_key(&ids[1])).await;
    eventually("b accepts", || links(&b).contains_key(&ids[0])).await;
    let before = (links(&a)[&ids[1]].clone(), links(&b)[&ids[0]].clone());
    assert_eq!(before.0.carrier.protocol, TransportProtocol::NativeTcp);
    let old_scope = a.shared.links.lock().unwrap()[&ids[1]].scope.clone();
    let acquisitions = factory.acquisitions.load(Ordering::SeqCst);
    let c_id = ids[2];
    let changed = fixture
        .publish(|cfg| cfg.nodes.get_mut(&c_id).unwrap().hostname = "renamed".into())
        .await;
    assert!(changed[0] && changed[1]);
    for (index, manager) in [(0, &mut a), (1, &mut b)] {
        tokio::time::timeout(
            Duration::from_secs(2),
            manager.apply(&fixture.desired[index]),
        )
        .await
        .unwrap()
        .unwrap();
    }
    assert!(
        old_scope.check().is_ok(),
        "a channel the new content still admits keeps its scope"
    );
    // The stream owner keeps receiving on its unchanged channel.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(links(&a)[&ids[1]], before.0);
    assert_eq!(links(&b)[&ids[0]], before.1);
    assert_eq!(factory.acquisitions.load(Ordering::SeqCst), acquisitions);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn a_pending_supersession_never_replaces_a_newer_channel_or_refills_a_retired_slot() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    a.interest().interest(ids[1]);
    eventually("b accepts a", || links(&b).contains_key(&ids[0])).await;
    let (current, sender) = {
        let links = b.shared.links.lock().unwrap();
        let link = &links[&ids[0]];
        (link.fact.clone(), link.sender.clone())
    };
    // A recovery captured against an older channel id (ABA: the slot was
    // since replaced by `current`).
    let mut stale = current.clone();
    stale.id = [0xab; 32];
    let ready = |supersedes| ReadyPeer {
        fact: stale.clone(),
        sender: sender.clone(),
        scope: b.shared.scope(Some((ids[0], stale.carrier.clone())), false),
        plan: b.shared.plan(),
        supersedes,
        attempt: None,
    };
    assert!(!install_ready(&b.shared, ready(Replacement::Channel([0xcd; 32]))).unwrap());
    assert_eq!(links(&b)[&ids[0]], current);
    // A healthy same-carrier UDP channel is never displaced by ranking alone.
    assert!(!install_ready(&b.shared, ready(Replacement::Ranked)).unwrap());
    assert_eq!(links(&b)[&ids[0]], current);
    // Once retired, the empty slot remembers its last channel: a recovery
    // that saw it empty earlier (no last channel) cannot refill it.
    retire(&b.shared, &current).unwrap();
    assert!(!links(&b).contains_key(&ids[0]));
    assert!(!install_ready(&b.shared, ready(Replacement::Vacant(None))).unwrap());
    assert!(!links(&b).contains_key(&ids[0]));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

/// A coordination host whose Control is unreachable: nothing is coordinated.
struct Offline;
impl crate::traversal::TraversalGateway for Offline {
    fn request(
        &self,
        _operation: crate::traversal::Operation,
    ) -> futures_util::future::BoxFuture<'_, Result<crate::traversal::Reply, Error>> {
        Box::pin(async { Err(Error::Unavailable) })
    }
    fn published(&self, _candidate: &meshway_protocol::connectivity::CandidateReference) -> bool {
        false
    }
}

#[tokio::test]
async fn dial_only_traversal_sockets_bind_once_the_first_instance_arrives() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let a = ids[0];
    // A dial-only Node (no listener to rebind) loaded offline, before Control
    // qualifies an instance: its traversal sockets need a State reference.
    fixture
        .publish(|cfg| cfg.nodes.get_mut(&a).unwrap().listeners = Default::default())
        .await;
    let factory = fixture.factory();
    let mut manager = Manager::start_with_traversal(
        &fixture.desired[0],
        fixture.keys[0].clone(),
        factory.clone(),
        Default::default(),
        Arc::new(Offline),
        meshway_model::Sequence::MIN.checked_next().unwrap(),
        None,
    )
    .await
    .unwrap();
    let dial_sockets = |manager: &Manager| {
        manager
            .snapshot()
            .unwrap()
            .sockets
            .iter()
            .filter(|socket| {
                matches!(
                    socket.reference.resource,
                    meshway_model::state::ResourceRef::Dial { .. }
                )
            })
            .count()
    };
    assert_eq!(dial_sockets(&manager), 0);
    manager
        .set_instance(Some(
            serde_json::from_value(serde_json::json!({
                "instanceId": "00000000-0000-4000-8000-0000000000c1",
                "generation": 1
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    assert!(
        dial_sockets(&manager) > 0,
        "dial-only traversal sockets bound"
    );
    assert!(factory.traversal_binds.load(Ordering::SeqCst) > 0);
    manager.shutdown().await.unwrap();
    fixture.close().await;
}

/// A Server restart ends every instance: the Node acquires a new one on the
/// same content. The Underlay authorization of each kept traversal socket
/// must name the new reference, or every later offer (prepared under the new
/// instance) is refused when it activates and this Node never punches.
#[tokio::test]
async fn a_replaced_instance_rebinds_every_traversal_socket() {
    let mut fixture = Netmaps::start(false, true).await;
    let a = fixture.ids()[0];
    fixture
        .publish(|cfg| cfg.nodes.get_mut(&a).unwrap().listeners = Default::default())
        .await;
    let factory = fixture.factory();
    let mut manager = Manager::start_with_traversal(
        &fixture.desired[0],
        fixture.keys[0].clone(),
        factory.clone(),
        Default::default(),
        Arc::new(Offline),
        meshway_model::Sequence::MIN.checked_next().unwrap(),
        None,
    )
    .await
    .unwrap();
    let instance = |n: u8| -> meshway_protocol::InstanceRequest {
        serde_json::from_value(serde_json::json!({
            "instanceId": format!("00000000-0000-4000-8000-0000000000c{n}"),
            "generation": 1
        }))
        .unwrap()
    };
    manager.set_instance(Some(instance(1))).await.unwrap();
    let sockets = manager.shared.traversal_endpoints.lock().unwrap().len();
    assert!(sockets > 0, "coordination sockets bound");
    factory.rebinds.lock().unwrap().clear();
    manager.set_instance(Some(instance(2))).await.unwrap();
    let rebinds = factory.rebinds.lock().unwrap().clone();
    assert_eq!(rebinds.len(), sockets, "every traversal socket rebound");
    assert!(
        rebinds
            .iter()
            .all(|reference| reference.instance == instance(2))
    );
    // The same instance again changes nothing.
    manager.set_instance(Some(instance(2))).await.unwrap();
    assert_eq!(factory.rebinds.lock().unwrap().len(), sockets);
    manager.shutdown().await.unwrap();
    fixture.close().await;
}
