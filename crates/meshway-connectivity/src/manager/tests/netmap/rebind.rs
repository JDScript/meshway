//! A listener whose port another process holds at start (ops5: nyc's
//! `0.0.0.0:35000` after a restart) is reported unavailable, retried by its
//! own owner and served once the port is free; the rest of the content runs.
use super::*;

fn address(desired: &Desired, key: &str) -> SocketAddr {
    let plan = crate::plan::Plan::new(desired).unwrap();
    match plan.listeners[key].declaration() {
        Listener::NativeUdp(value) => SocketAddr::new(value.listen, value.listen_port.get()),
        Listener::NativeTcp(value) => SocketAddr::new(value.listen, value.listen_port.get()),
        _ => unreachable!(),
    }
}

async fn bound_after_the_port_is_released(stream: bool) {
    let mut fixture = Netmaps::start(stream, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let key = if stream { "tcp" } else { "udp" };
    let held = address(&fixture.desired[1], key);
    // Another process holds b's port: the reserved socket leaves the factory.
    let holder: Box<dyn std::any::Any + Send> = if stream {
        Box::new(factory.tcp.lock().unwrap().remove(&held).unwrap())
    } else {
        Box::new(factory.udp.lock().unwrap().remove(&held).unwrap())
    };
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    let unavailable = b.unavailable_listeners();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].key, key);
    assert_eq!(unavailable[0].address, Some(held));
    assert!(unavailable[0].recoverable);
    assert!(
        unavailable[0]
            .detail
            .starts_with(&format!("listener {key} cannot bind {held}: "))
    );
    let snapshot = b.snapshot().unwrap();
    assert!(snapshot.listeners.is_empty());
    assert_eq!(snapshot.unavailable, unavailable);
    // Still unbound after the first retry (1 s) while the port is held.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1800);
    while let Ok(event) = tokio::time::timeout_at(deadline, b.recv()).await {
        assert!(!matches!(event, Event::Closed), "the Manager keeps running");
    }
    let retried = b.unavailable_listeners();
    assert_eq!(retried.len(), 1);
    assert!(retried[0].attempts >= 2, "{retried:?}");
    drop(holder);
    // The next attempt binds; the Manager serves it and says so.
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            match b.recv().await {
                Event::Closed => panic!("the Manager keeps running"),
                Event::Changed if b.unavailable_listeners().is_empty() => break,
                _ => {}
            }
        }
    })
    .await
    .expect("rebound listener adopted");
    let snapshot = b.snapshot().unwrap();
    assert!(snapshot.unavailable.is_empty());
    assert_eq!(snapshot.listeners.len(), 1);
    assert_eq!(snapshot.listeners[0].local, held);
    // The retried bind reads its granted buffers back like a first bind.
    assert_eq!(snapshot.listeners[0].buffers.is_some(), !stream);
    // And it is a real listener: a reaches b through it.
    a.interest().interest(ids[1]);
    eventually("a reaches b on the rebound listener", || {
        links(&a).contains_key(&ids[1])
    })
    .await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn a_held_udp_listener_port_is_retried_and_served_once_free() {
    bound_after_the_port_is_released(false).await;
}

#[tokio::test]
async fn a_held_tcp_listener_port_is_retried_and_served_once_free() {
    bound_after_the_port_is_released(true).await;
}

#[tokio::test]
async fn new_content_without_the_listener_ends_its_retry() {
    let mut fixture = Netmaps::start(false, true).await;
    let b = fixture.ids()[1];
    let factory = fixture.factory();
    let held = address(&fixture.desired[1], "udp");
    let _holder = factory.udp.lock().unwrap().remove(&held).unwrap();
    let mut manager = manager(&fixture, 1, &factory, Default::default()).await;
    assert_eq!(manager.unavailable_listeners().len(), 1);
    fixture
        .publish(|cfg| cfg.nodes.get_mut(&b).unwrap().listeners = Default::default())
        .await;
    manager.apply(&fixture.desired[1]).await.unwrap();
    assert!(manager.unavailable_listeners().is_empty());
    assert!(manager.snapshot().unwrap().unavailable.is_empty());
    manager.shutdown().await.unwrap();
    fixture.close().await;
}

/// b's UDP port is held at start; once freed, the retry owner binds and
/// hands the socket over, but the Manager has not adopted it yet (nothing
/// reads `recv`). That queued bind is the old socket of the key.
async fn queued_bind() -> (Manager, Netmaps, SocketAddr) {
    let mut fixture = Netmaps::start(false, true).await;
    let factory = fixture.factory();
    let held = address(&fixture.desired[1], "udp");
    let holder = factory.udp.lock().unwrap().remove(&held).unwrap();
    let b = manager(&fixture, 1, &factory, Default::default()).await;
    assert_eq!(b.unavailable_listeners().len(), 1);
    drop(holder);
    // The first retry runs after 1 s and binds.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    (b, fixture, held)
}

/// New content that changes the listener's declaration on the same port
/// retires the retry owner; its unadopted bind is closed before the new
/// declaration binds, so the bind does not meet the old socket.
#[tokio::test]
async fn an_unadopted_bind_is_closed_before_new_content_binds_its_port() {
    let (mut b, mut fixture, held) = queued_bind().await;
    let id = fixture.ids()[1];
    fixture
        .publish(|cfg| {
            let mut listener = super::support::udp_listener(held, true);
            if let Listener::NativeUdp(value) = &mut listener {
                value.socket.receive_buffer = 1 << 20;
            }
            cfg.nodes.get_mut(&id).unwrap().listeners.insert(
                "udp".to_owned(),
                meshway_transport::spec::PreparedListener::prepare(listener).unwrap(),
            );
        })
        .await;
    b.apply(&fixture.desired[1]).await.unwrap();
    assert_eq!(b.unavailable_listeners(), Vec::new());
    let snapshot = b.snapshot().unwrap();
    assert_eq!(snapshot.listeners.len(), 1);
    assert_eq!(snapshot.listeners[0].local, held);
    b.shutdown().await.unwrap();
    fixture.close().await;
}

/// Shutdown closes an unadopted bind before it returns (a restart binds the
/// same port before the old Manager value is replaced).
#[tokio::test]
async fn shutdown_closes_an_unadopted_bind() {
    let (mut b, fixture, held) = queued_bind().await;
    b.shutdown().await.unwrap();
    let rebound = std::net::UdpSocket::bind(held);
    assert!(rebound.is_ok(), "{rebound:?}");
    drop(rebound);
    drop(b);
    fixture.close().await;
}

fn instance(n: u8) -> meshway_protocol::InstanceRequest {
    serde_json::from_value(serde_json::json!({
        "instanceId": format!("00000000-0000-4000-8000-0000000000c{n}"),
        "generation": 1
    }))
    .unwrap()
}

/// b's UDP listener port is held while b runs offline under a traversal
/// gateway; it is freed and the retry owner binds, then an instance arrives
/// before the Manager adopts that bind (`instances` in order). The adopted
/// listener is what a bind under the last instance would be.
async fn adopted_after(instances: &[u8]) -> (Manager, Arc<Factory>, Netmaps, SocketAddr) {
    let mut fixture = Netmaps::start(false, true).await;
    let factory = fixture.factory();
    let held = address(&fixture.desired[1], "udp");
    let holder = factory.udp.lock().unwrap().remove(&held).unwrap();
    let mut b = Manager::start_with_traversal(
        &fixture.desired[1],
        fixture.keys[1].clone(),
        factory.clone(),
        Default::default(),
        Arc::new(Offline),
        meshway_model::Sequence::MIN.checked_next().unwrap(),
        None,
        None,
    )
    .await
    .unwrap();
    let (last, before) = instances.split_last().unwrap();
    for n in before {
        b.set_instance(Some(instance(*n))).await.unwrap();
    }
    assert_eq!(b.unavailable_listeners().len(), 1);
    drop(holder);
    // The retry owner binds after 1 s and waits for the Manager to adopt.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    b.set_instance(Some(instance(*last))).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match b.recv().await {
                Event::Closed => panic!("the Manager keeps running"),
                Event::Changed if b.unavailable_listeners().is_empty() => break,
                _ => {}
            }
        }
    })
    .await
    .expect("rebound listener adopted");
    (b, factory, fixture, held)
}

/// The U1 start: b's plain listener bound offline is rebound as a
/// coordination socket when the first instance arrives. The plain socket's
/// owner is joined (its socket closed) before the same port binds again.
#[tokio::test]
async fn the_first_instance_rebinds_an_installed_plain_listener_on_its_port() {
    let mut fixture = Netmaps::start(false, true).await;
    let factory = fixture.factory();
    let held = address(&fixture.desired[1], "udp");
    let mut b = Manager::start_with_traversal(
        &fixture.desired[1],
        fixture.keys[1].clone(),
        factory.clone(),
        Default::default(),
        Arc::new(Offline),
        meshway_model::Sequence::MIN.checked_next().unwrap(),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(factory.traversal_listeners.load(Ordering::SeqCst), 0);
    assert_eq!(b.snapshot().unwrap().listeners[0].local, held);
    // The reserved socket is gone: the rebind is a real bind of `held`.
    assert!(factory.udp.lock().unwrap().get(&held).is_none());
    b.set_instance(Some(instance(1))).await.unwrap();
    assert_eq!(b.unavailable_listeners(), Vec::new());
    assert_eq!(factory.traversal_listeners.load(Ordering::SeqCst), 1);
    assert_eq!(b.snapshot().unwrap().listeners[0].local, held);
    assert_eq!(listener_reference(&b).instance, instance(1));
    b.shutdown().await.unwrap();
    fixture.close().await;
}

fn listener_reference(manager: &Manager) -> meshway_protocol::connectivity::SocketReference {
    let snapshot = manager.snapshot().unwrap();
    assert_eq!(snapshot.listeners.len(), 1);
    let id = snapshot.listeners[0].socket_id;
    assert!(
        manager
            .shared
            .traversal_endpoints
            .lock()
            .unwrap()
            .contains_key(&id),
        "the listener is a coordination socket"
    );
    snapshot
        .sockets
        .into_iter()
        .find(|socket| {
            matches!(
                &socket.reference.resource,
                meshway_model::state::ResourceRef::Listener { key } if key == "udp"
            )
        })
        .unwrap()
        .reference
}

/// Bound plain before any instance: adopted as a coordination socket on the
/// same port (the plain socket is released first, so the new bind does not
/// meet its own port).
#[tokio::test]
async fn a_plain_bind_adopted_after_the_first_instance_becomes_a_coordination_socket() {
    let (mut b, factory, fixture, held) = adopted_after(&[1]).await;
    assert_eq!(factory.traversal_listeners.load(Ordering::SeqCst), 1);
    assert_eq!(b.snapshot().unwrap().listeners[0].local, held);
    assert_eq!(listener_reference(&b).instance, instance(1));
    b.shutdown().await.unwrap();
    fixture.close().await;
}

/// Bound as a coordination socket under instance 1, adopted after instance 2:
/// its reference and Underlay authorization name instance 2.
#[tokio::test]
async fn a_coordination_bind_adopted_after_a_new_instance_names_it() {
    let (mut b, factory, fixture, _) = adopted_after(&[1, 2]).await;
    assert_eq!(factory.traversal_listeners.load(Ordering::SeqCst), 1);
    assert_eq!(listener_reference(&b).instance, instance(2));
    assert!(
        factory.rebinds.lock().unwrap().iter().any(|reference| {
            reference.instance == instance(2)
                && matches!(
                    &reference.resource,
                    meshway_model::state::ResourceRef::Listener { key } if key == "udp"
                )
        }),
        "the authorization was rebound to instance 2"
    );
    b.shutdown().await.unwrap();
    fixture.close().await;
}

type Change = Box<dyn FnOnce(&mut meshway_configuration::NetworkConfiguration)>;

/// X12/R2: a configuration change applied to Managers serving traversal
/// sockets and a live channel (a listener of this Node removed and added
/// again, as in the R1 stage change, and an unrelated rename) is applied in
/// place: every apply succeeds, the Manager keeps running and its boot, so
/// the network worker never restarts adjacencies (a new boot to every peer).
#[tokio::test]
async fn configuration_changes_with_live_traversal_channels_keep_the_manager_and_its_boot() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let instance = |n: u8| -> meshway_protocol::InstanceRequest {
        serde_json::from_value(serde_json::json!({
            "instanceId": format!("00000000-0000-4000-8000-0000000000d{n}"),
            "generation": 1
        }))
        .unwrap()
    };
    let mut managers = Vec::new();
    for index in 0..2 {
        let mut manager = Manager::start_with_traversal(
            &fixture.desired[index],
            fixture.keys[index].clone(),
            factory.clone(),
            Default::default(),
            Arc::new(super::Offline),
            meshway_model::Sequence::MIN.checked_next().unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        manager
            .set_instance(Some(instance(index as u8 + 1)))
            .await
            .unwrap();
        managers.push(manager);
    }
    let boots: Vec<_> = managers.iter().map(Manager::boot).collect();
    managers[0].interest().interest(ids[1]);
    eventually("a links b", || links(&managers[0]).contains_key(&ids[1])).await;
    let (a, c) = (ids[0], ids[2]);
    let listener = fixture.cfg.nodes[&a].listeners.clone();
    for round in 0..3 {
        let listener = listener.clone();
        let changes: [Change; 3] = [
            Box::new(move |cfg| cfg.nodes.get_mut(&a).unwrap().listeners = Default::default()),
            Box::new(move |cfg| cfg.nodes.get_mut(&a).unwrap().listeners = listener.clone()),
            Box::new(move |cfg| {
                cfg.nodes.get_mut(&c).unwrap().hostname = format!("renamed-{round}")
            }),
        ];
        for change in changes {
            fixture.publish(change).await;
            for (index, manager) in managers.iter_mut().enumerate() {
                let applied = tokio::time::timeout(
                    Duration::from_secs(10),
                    manager.apply(&fixture.desired[index]),
                )
                .await
                .unwrap();
                assert!(applied.is_ok(), "round {round} node {index}: {applied:?}");
                assert!(manager.is_running(), "round {round} node {index}");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(managers.iter().all(Manager::is_running), "round {round}");
        }
        managers[0].interest().interest(ids[1]);
        eventually("a links b again", || {
            links(&managers[0]).contains_key(&ids[1])
        })
        .await;
    }
    assert_eq!(
        managers.iter().map(Manager::boot).collect::<Vec<_>>(),
        boots
    );
    // The network worker's restart after a failed apply carries the boot
    // while its packet owner keeps the circuits: the peer sees the same one.
    managers[0].shutdown().await.unwrap();
    managers[0] = Manager::start_with_traversal(
        &fixture.desired[0],
        fixture.keys[0].clone(),
        factory.clone(),
        Default::default(),
        Arc::new(super::Offline),
        meshway_model::Sequence::MIN.checked_next().unwrap(),
        Some(instance(1)),
        Some(boots[0]),
    )
    .await
    .unwrap();
    assert_eq!(managers[0].boot(), boots[0]);
    managers[0].interest().interest(ids[1]);
    eventually("b sees a's new channel with the same boot", || {
        let own = links(&managers[0]).get(&ids[1]).map(|link| link.id);
        own.is_some()
            && links(&managers[1]).get(&ids[0]).map(|link| link.id) == own
            && managers[1]
                .peer_boots()
                .is_ok_and(|peers| peers.get(&ids[0]) == Some(&boots[0]))
    })
    .await;
    for mut manager in managers {
        manager.shutdown().await.unwrap();
    }
    fixture.close().await;
}
