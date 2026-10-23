//! The Manager's own part of a path change: listeners bound to an address
//! the host lost, their bind retry, and the remotes the host's change
//! classification follows.
use super::*;

fn address(desired: &Desired, key: &str) -> SocketAddr {
    let plan = crate::plan::Plan::new(desired).unwrap();
    match plan.listeners[key].declaration() {
        Listener::NativeUdp(value) => SocketAddr::new(value.listen, value.listen_port.get()),
        Listener::NativeTcp(value) => SocketAddr::new(value.listen, value.listen_port.get()),
        _ => unreachable!(),
    }
}

/// Drive `manager` until `ready` holds after a `Changed`, within `limit`.
async fn changed_until(manager: &mut Manager, limit: Duration, ready: impl Fn(&Manager) -> bool) {
    tokio::time::timeout(limit, async {
        loop {
            match manager.recv().await {
                Event::Closed => panic!("the Manager keeps running"),
                Event::Changed if ready(manager) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("changed in time");
}

/// b's listener is bound to a specific address (`127.0.0.1`) that leaves the
/// host: the path change closes it and its bind is retried, backing off,
/// never ending in a permanent unavailability; the path change that brings
/// the address back binds it at once rather than after the grown backoff.
async fn a_departed_listener_address(stream: bool) {
    let mut fixture = Netmaps::start(stream, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let key = if stream { "tcp" } else { "udp" };
    let held = address(&fixture.desired[1], key);
    assert!(!held.ip().is_unspecified());
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    assert_eq!(b.path_changed().await.unwrap().listeners_unbound, 0);
    assert_eq!(b.snapshot().unwrap().listeners.len(), 1);
    factory.absent.lock().unwrap().insert(held.ip());
    assert_eq!(b.path_changed().await.unwrap().listeners_unbound, 1);
    let unavailable = b.unavailable_listeners();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].key, key);
    assert_eq!(unavailable[0].address, Some(held));
    assert!(unavailable[0].recoverable());
    assert!(b.snapshot().unwrap().listeners.is_empty());
    // Away for 3.5 s: retried at 1 s and 3 s, the next attempt 4 s later.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(3500);
    while let Ok(event) = tokio::time::timeout_at(deadline, b.recv()).await {
        assert!(!matches!(event, Event::Closed), "the Manager keeps running");
    }
    let retried = b.unavailable_listeners();
    assert_eq!(retried.len(), 1, "still retried, not given up");
    assert!(retried[0].attempts >= 3, "{retried:?}");
    assert!(
        retried[0].retry_at > std::time::Instant::now() + Duration::from_secs(2),
        "{retried:?}"
    );
    // The address is back; the path change runs the bind now.
    factory.absent.lock().unwrap().clear();
    let back = std::time::Instant::now();
    assert_eq!(b.path_changed().await.unwrap().listeners_unbound, 0);
    changed_until(&mut b, Duration::from_millis(1500), |b| {
        b.unavailable_listeners().is_empty()
    })
    .await;
    assert!(back.elapsed() < Duration::from_millis(1500));
    let snapshot = b.snapshot().unwrap();
    assert_eq!(snapshot.listeners.len(), 1);
    assert_eq!(snapshot.listeners[0].local, held);
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
async fn a_udp_listener_whose_address_left_is_retried_and_bound_when_it_returns() {
    a_departed_listener_address(false).await;
}

#[tokio::test]
async fn a_tcp_listener_whose_address_left_is_retried_and_bound_when_it_returns() {
    a_departed_listener_address(true).await;
}

/// The host learns the remote of every selected channel: a route change
/// toward only that peer is a path change for the host's classification.
#[tokio::test]
async fn the_host_follows_the_remote_of_every_selected_channel() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let mut a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    a.interest().interest(ids[1]);
    eventually("a reaches b", || links(&a).contains_key(&ids[1])).await;
    let remote = links(&a)[&ids[1]].remote.ip();
    changed_until(&mut a, Duration::from_secs(5), |_| {
        factory
            .remotes
            .lock()
            .unwrap()
            .last()
            .is_some_and(|remotes| remotes.first() == Some(&remote))
    })
    .await;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}
