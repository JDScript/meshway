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
