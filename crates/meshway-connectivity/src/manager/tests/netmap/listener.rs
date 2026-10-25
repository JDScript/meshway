//! L1: one listener's failure stays that listener's. Field (it7, alpha.23):
//! the public Native TCP listener's owner ended on an accept error and
//! stopped the whole Manager, withdrawing the packet plane for every flow
//! through the exit. Red before: both tests saw `Event::Closed`.
use super::*;
use crate::manager::tests::support::AcceptFault;

/// b's Manager events until `deadline`; it never closes.
async fn serve_until(manager: &mut Manager, deadline: tokio::time::Instant) {
    while let Ok(event) = tokio::time::timeout_at(deadline, manager.recv()).await {
        assert!(!matches!(event, Event::Closed), "the Manager keeps running");
    }
}

/// Serve b (it never closes) until `ready`, within `within`.
async fn serve_until_ready(
    b: &mut Manager,
    what: &str,
    within: Duration,
    mut ready: impl FnMut(&Manager) -> bool,
) {
    let deadline = tokio::time::Instant::now() + within;
    while !ready(b) {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        serve_until(b, tokio::time::Instant::now() + Duration::from_millis(20)).await;
    }
}

/// a and c dial b's only (Native TCP) listener; both edges are up.
async fn connected() -> (Netmaps, SocketAddr, Arc<Factory>, [Manager; 3]) {
    let mut fixture = Netmaps::start(true, true).await;
    let ids = fixture.ids();
    let address = fixture.tcp_address();
    let factory = fixture.factory();
    let a = manager(&fixture, 0, &factory, Default::default()).await;
    let b = manager(&fixture, 1, &factory, Default::default()).await;
    let c = manager(&fixture, 2, &factory, Default::default()).await;
    for (from, to) in [(&a, ids[1]), (&c, ids[1]), (&b, ids[0]), (&b, ids[2])] {
        eventually("the edge toward b is up", || links(from).contains_key(&to)).await;
    }
    (fixture, address, factory, [a, b, c])
}

/// The owner reaches the injected failures at its next accept: one plain
/// connection (a scanner's, no handshake) completes the pending one.
async fn knock(address: SocketAddr) {
    drop(TcpStream::connect(address).await.unwrap());
}

/// Host errors from accept(2) (descriptors, buffers exhausted) are retried
/// by the listener's owner: the Manager keeps running, the established
/// channels through the listener stay, and it accepts the next connection.
#[tokio::test]
async fn failed_accepts_are_retried_without_stopping_the_manager() {
    let (fixture, address, factory, [mut a, mut b, mut c]) = connected().await;
    let ids = fixture.ids();
    let before = (links(&a)[&ids[1]].id, links(&c)[&ids[1]].id);
    // EMFILE, ENFILE, ENOBUFS, EMFILE, EMFILE: 50+100+200+400+800 ms apart.
    factory
        .faults
        .accept_faults
        .lock()
        .unwrap()
        .extend([24, 23, 105, 24, 24].map(AcceptFault::Host));
    knock(address).await;
    serve_until(
        &mut b,
        tokio::time::Instant::now() + Duration::from_millis(2500),
    )
    .await;
    assert!(factory.faults.accept_faults.lock().unwrap().is_empty());
    assert!(b.unavailable_listeners().is_empty());
    assert_eq!(
        (links(&a)[&ids[1]].id, links(&c)[&ids[1]].id),
        before,
        "the channels through the listener are untouched"
    );
    // And it still accepts: c's next Manager dials b again.
    c.shutdown().await.unwrap();
    let mut c = manager(&fixture, 2, &factory, Default::default()).await;
    serve_until_ready(&mut b, "c reaches b again", Duration::from_secs(12), |_| {
        links(&c).contains_key(&ids[1])
    })
    .await;
    assert_eq!(links(&a)[&ids[1]].id, before.0);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

/// The listener's socket itself ends: only that listener is rebound (by
/// its retry owner, reported unavailable meanwhile). The Manager keeps
/// running and the dialers reach b through the new socket.
#[tokio::test]
async fn an_ended_listener_socket_is_rebound_alone() {
    let (fixture, address, factory, [mut a, mut b, mut c]) = connected().await;
    let ids = fixture.ids();
    let before = links(&a)[&ids[1]].id;
    factory
        .faults
        .accept_faults
        .lock()
        .unwrap()
        .push_back(AcceptFault::Ended);
    knock(address).await;
    serve_until_ready(
        &mut b,
        "the failed listener is reported while it is rebound",
        Duration::from_secs(5),
        |b| !b.unavailable_listeners().is_empty(),
    )
    .await;
    let unavailable = b.unavailable_listeners();
    assert_eq!(unavailable[0].key, "tcp");
    assert_eq!(unavailable[0].detail, Error::Underlay.to_string());
    serve_until_ready(
        &mut b,
        "b's listener is rebound and both dialers reach it",
        Duration::from_secs(15),
        |b| {
            b.unavailable_listeners().is_empty()
                && links(&a).get(&ids[1]).is_some_and(|link| link.id != before)
                && links(&c).contains_key(&ids[1])
        },
    )
    .await;
    let snapshot = b.snapshot().unwrap();
    assert_eq!(snapshot.listeners.len(), 1);
    assert_eq!(snapshot.listeners[0].local, address);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
    fixture.close().await;
}

/// Host errors receiving on a Native UDP listener socket (ENOMEM, ENOBUFS)
/// pause its reads by the same bounded backoff; the Manager keeps running
/// and the channel on the socket stays.
#[tokio::test]
async fn failed_receives_on_a_udp_listener_are_retried_without_stopping_the_manager() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let factory = fixture.factory();
    let plan = crate::plan::Plan::new(&fixture.desired[1]).unwrap();
    let Listener::NativeUdp(declaration) = plan.listeners["udp"].declaration() else {
        unreachable!()
    };
    let socket = SocketAddr::new(declaration.listen, declaration.listen_port.get());
    let a = manager(&fixture, 0, &factory, Default::default()).await;
    let mut b = manager(&fixture, 1, &factory, Default::default()).await;
    a.interest().interest(ids[1]);
    serve_until_ready(&mut b, "a reaches b", Duration::from_secs(12), |b| {
        links(&a).contains_key(&ids[1]) && links(b).contains_key(&ids[0])
    })
    .await;
    let before = (links(&a)[&ids[1]].id, links(&b)[&ids[0]].id);
    factory
        .faults
        .receive_faults
        .lock()
        .unwrap()
        .insert(socket, [12, 105, 12].into());
    // The owner meets them at its next read.
    let knock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    knock.send_to(b"knock", socket).await.unwrap();
    serve_until(
        &mut b,
        tokio::time::Instant::now() + Duration::from_millis(1500),
    )
    .await;
    assert!(factory.faults.receive_faults.lock().unwrap()[&socket].is_empty());
    assert!(b.unavailable_listeners().is_empty());
    assert_eq!(
        (links(&a)[&ids[1]].id, links(&b)[&ids[0]].id),
        before,
        "the channel on the socket is untouched"
    );
    let mut a = a;
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    fixture.close().await;
}

/// The `reason` of each WARN on this thread.
struct Reasons(Arc<std::sync::Mutex<Vec<String>>>);
struct Reason<'a>(&'a mut Option<String>);
impl tracing::field::Visit for Reason<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "reason" {
            *self.0 = Some(value.to_owned());
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "reason" {
            *self.0 = Some(format!("{value:?}"));
        }
    }
}
impl tracing::Subscriber for Reasons {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut reason = None;
        event.record(&mut Reason(&mut reason));
        if *event.metadata().level() == tracing::Level::WARN
            && let Some(reason) = reason
        {
            self.0.lock().unwrap().push(reason);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// L1: an owner that ends unasked stops the Manager and says why: the
/// error it ended with, or its panic. Before, the WARN named only the
/// owner (it7: `Listener("prod-tcp")` and nothing else).
#[tokio::test]
async fn an_owner_ending_unrequested_says_why() {
    let reasons = Arc::new(std::sync::Mutex::new(Vec::new()));
    let _guard = tracing::subscriber::set_default(Reasons(reasons.clone()));
    let mut fixture = Netmaps::start(false, true).await;
    let factory = fixture.factory();
    for (ending, reason) in [
        (
            Err(Error::Io(std::io::Error::from_raw_os_error(24))),
            "failed: transport I/O failed: ",
        ),
        (Ok(()), "returned"),
        (Err(Error::Worker), "panicked: owner bug"),
    ] {
        let mut manager = Manager::start(
            &fixture.desired[1],
            fixture.keys[1].clone(),
            factory.clone(),
            Default::default(),
        )
        .await
        .unwrap();
        let panics = matches!(ending, Err(Error::Worker));
        manager.spawn_owner(
            crate::manager::resources::Key::Traversal,
            None,
            move |_| async move {
                if panics {
                    panic!("owner bug");
                }
                ending
            },
        );
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if matches!(manager.recv().await, Event::Closed) {
                    break;
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "the Manager stops");
        let warned = reasons.lock().unwrap().pop().unwrap_or_default();
        assert!(warned.starts_with(reason), "{warned:?} names {reason:?}");
        let _ = manager.shutdown().await;
        // The listener's port is free again for the next Manager.
        tokio::task::yield_now().await;
    }
    fixture.close().await;
}
