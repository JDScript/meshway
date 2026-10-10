//! Desired netmap content: on-demand adjacency, idle close, resident edges,
//! peer facts without reconfiguration and per-peer serial differentials, over
//! real loopback sockets and Control-verified netmaps.
use super::*;
use meshway_configuration::{TopologyAction, TopologyRule};
use meshway_control::PeerFactsSink;
use meshway_model::state::{ListenerPhase, LocalEndpoint, LocalEndpointSource, SocketNetwork};
use meshway_protocol::{PeerFacts, PeerFactsDelta, PeerListenerFacts, StateCursor};
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
    eventually("keepalive edge acquires", || {
        links(&a).contains_key(&ids[1])
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
        observed_at: now,
        expires_at: now + chrono::Duration::minutes(5),
    }
}

fn delta(base: Option<u64>, cursor: u64, updated: Vec<PeerFacts>) -> PeerFactsDelta {
    let epoch = "00000000-0000-4000-8000-000000000077".parse().unwrap();
    let cursor_at = |n| StateCursor {
        epoch,
        sequence: meshway_model::Sequence::new(n).unwrap(),
    };
    PeerFactsDelta {
        base: base.map(cursor_at),
        cursor: cursor_at(cursor),
        netmap_serial: meshway_model::Version::MIN,
        updated: updated
            .into_iter()
            .map(|facts| (facts.node_id, facts))
            .collect(),
        removed: vec![],
    }
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
    let mut sink = a.facts_sink();
    sink.apply(netmap, delta(None, 1, vec![facts(ids[1], b_address)]))
        .unwrap();
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
    sink.apply(netmap, delta(Some(1), 2, vec![])).unwrap();
    let mut removed = delta(Some(2), 3, vec![]);
    removed.removed.push(ids[1]);
    sink.apply(netmap, removed).unwrap();
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
    a.facts_sink()
        .apply(netmap, delta(None, 1, vec![stale]))
        .unwrap();
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
