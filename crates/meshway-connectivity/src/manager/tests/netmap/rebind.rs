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
