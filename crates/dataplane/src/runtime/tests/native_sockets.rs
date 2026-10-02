use super::*;
use crate::sockets::{
    NativeListener, NativeSocketFactory, NativeSocketPurpose, NativeTcp, NativeUdp, SocketFuture,
    SystemSocketFactory,
};
use std::{io, sync::atomic::AtomicUsize};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Deliberately pause synchronous dispatch on its real mutex; test-only block_in_place releases the worker queue.
async fn rebind_waits_for_admitted_native_dispatch_and_never_delivers_old_ingress_later() {
    for transport in [TransportKind::NativeUdp, TransportKind::NativeTcp] {
        let direct = vec!["nyc".into(), "lax".into()];
        let mut config = configs();
        for node in &mut config {
            node.allowed_paths.push(direct.clone());
        }
        config[1].listeners.push(ListenerConfig {
            transport,
            bind: "127.0.0.1:0".parse().unwrap(),
            id: None,
            allowed_peers: None,
        });
        let server = Runtime::start(config[1].clone()).await.unwrap();
        config[0].outgoing.push(OutgoingCandidate {
            transport,
            endpoint: server.bound_listeners()[0].address.to_string(),
            peer: "lax".into(),
            priority: 0,
            id: None,
        });
        let client = Runtime::start(config[0].clone()).await.unwrap();
        client
            .wait_for_peer("lax", Duration::from_secs(3))
            .await
            .unwrap();
        let mut delivery = server.take_packet_receiver().unwrap();
        client
            .send_packet(direct.clone(), b"warmup".to_vec())
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), delivery.recv())
                .await
                .unwrap()
                .unwrap()
                .payload,
            b"warmup"
        );
        let packet_generation = server.handle.inner.packet_generation();
        let socket_generation = server.handle.native.lock().await.generation.0.clone();
        server
            .handle
            .inner
            .blocking_dispatch_test
            .store(true, Ordering::Relaxed);
        // Exactly the independent review interleaving: real authenticated bytes
        // have passed active/token + packet admission, then block on endpoints.
        let endpoints = lock(&server.handle.inner.endpoints);
        client
            .send_packet(direct.clone(), b"old-carrier-payload".to_vec())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !packet_generation.admission_held_for_test() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(socket_generation.admission_held_for_test());
        let clone = server.clone();
        let rebind = tokio::spawn(async move {
            clone
                .replace_native_socket_factory(Arc::new(SystemSocketFactory))
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let returned_before_commit = rebind.is_finished();
        assert!(delivery.try_recv().is_err());
        drop(endpoints);
        tokio::time::timeout(Duration::from_secs(2), rebind)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            !returned_before_commit,
            "{transport:?}: rebind returned while old dispatch was still uncommitted"
        );
        // Admitted dispatch may commit before retirement completes, never after.
        assert_eq!(delivery.try_recv().unwrap().payload, b"old-carrier-payload");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), delivery.recv())
                .await
                .is_err()
        );
        assert!(Arc::ptr_eq(
            &packet_generation,
            &server.handle.inner.packet_generation()
        ));
        client
            .replace_native_socket_factory(Arc::new(SystemSocketFactory))
            .await
            .unwrap();
        client
            .wait_for_peer("lax", Duration::from_secs(3))
            .await
            .unwrap();
        client
            .send_packet(direct, b"new-carrier-payload".to_vec())
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), delivery.recv())
                .await
                .unwrap()
                .unwrap()
                .payload,
            b"new-carrier-payload"
        );
        tokio::join!(client.shutdown(), server.shutdown());
    }
}

#[derive(Default)]
struct Recording {
    udp: AtomicUsize,
    listen: AtomicUsize,
    dial: AtomicUsize,
    fail: bool,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)] // Same deliberate cross-thread dispatch pause as the independent endpoint counterexample.
async fn rebind_waits_for_admitted_native_forwarding_commit() {
    for transport in [TransportKind::NativeUdp, TransportKind::NativeTcp] {
        let fixture = Fixture::native(transport).await;
        let mut delivery = fixture.sha.take_packet_receiver().unwrap();
        fixture
            .nyc
            .send_packet(path(), b"warmup".to_vec())
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), delivery.recv())
                .await
                .unwrap()
                .unwrap()
                .payload,
            b"warmup"
        );
        let inner = fixture.lax.handle.inner.clone();
        let packet_generation = inner.packet_generation();
        inner.blocking_dispatch_test.store(true, Ordering::Relaxed);
        // This real relay dispatch pauses immediately before next-hop enqueue.
        let transit = lock(&inner.observed_transit);
        let before = inner.counters(transport).forwarded.load(Ordering::Relaxed);
        fixture
            .nyc
            .send_packet(path(), b"old-transit".to_vec())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !packet_generation.admission_held_for_test() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let relay = fixture.lax.clone();
        let rebind = tokio::spawn(async move {
            relay
                .replace_native_socket_factory(Arc::new(SystemSocketFactory))
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let returned_before_commit = rebind.is_finished();
        assert_eq!(
            inner.counters(transport).forwarded.load(Ordering::Relaxed),
            before
        );
        drop(transit);
        tokio::time::timeout(Duration::from_secs(2), rebind)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            !returned_before_commit,
            "{transport:?}: rebind crossed pending forwarding admission"
        );
        assert_eq!(
            inner.counters(transport).forwarded.load(Ordering::Relaxed),
            before + 1
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            inner.counters(transport).forwarded.load(Ordering::Relaxed),
            before + 1
        );
        assert!(Arc::ptr_eq(&packet_generation, &inner.packet_generation()));
        fixture.shutdown().await;
    }
}
impl NativeSocketFactory for Recording {
    fn udp(
        &self,
        bind: SocketAddr,
        buffer: usize,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>> {
        self.udp.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(io::Error::other("injected setup failure"));
        }
        SystemSocketFactory.udp(bind, buffer, purpose)
    }
    fn listen(
        &self,
        bind: SocketAddr,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>> {
        self.listen.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(io::Error::other("injected setup failure"));
        }
        SystemSocketFactory.listen(bind, purpose)
    }
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        purpose: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>> {
        self.dial.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.fail {
                return Err(io::Error::other("injected connect failure"));
            }
            SystemSocketFactory.dial(endpoint, purpose).await
        })
    }
}

#[tokio::test]
async fn injected_native_factory_rebinds_same_runtime_and_failure_stays_closed() {
    for transport in [TransportKind::NativeUdp, TransportKind::NativeTcp] {
        let mut config = configs();
        config[1].listeners.push(ListenerConfig {
            transport,
            bind: "127.0.0.1:0".parse().unwrap(),
            id: Some("native-in".into()),
            allowed_peers: None,
        });
        let server_factory = Arc::new(Recording::default());
        let server = Runtime::start_with_socket_factory(config[1].clone(), server_factory.clone())
            .await
            .unwrap();
        let bound = server.bound_listeners()[0].address;
        config[0].outgoing.push(OutgoingCandidate {
            peer: "lax".into(),
            transport,
            endpoint: bound.to_string(),
            priority: 0,
            id: Some("native-out".into()),
        });
        let client_factory = Arc::new(Recording::default());
        let client = Runtime::start_with_socket_factory(config[0].clone(), client_factory.clone())
            .await
            .unwrap();
        client
            .wait_for_peer("lax", Duration::from_secs(3))
            .await
            .unwrap();
        server
            .wait_for_peer("nyc", Duration::from_secs(3))
            .await
            .unwrap();
        client
            .probe_neighbor("lax", Duration::from_secs(1))
            .await
            .unwrap();
        let old_packet = server.handle.inner.packet_generation();
        old_packet.retire();
        let failed = Arc::new(Recording {
            fail: true,
            ..Default::default()
        });
        assert!(
            server
                .replace_native_socket_factory(failed.clone())
                .await
                .is_err()
        );
        assert!(server.connected_peers().is_empty());
        assert!(server.udp_sockets().is_empty());
        // The previously assigned listener is closed, not silently rebound by a
        // default system factory after the injected failure.
        if transport == TransportKind::NativeTcp {
            assert!(tokio::net::TcpStream::connect(bound).await.is_err());
        } else {
            let reclaim = UdpSocket::bind(bound).await.unwrap();
            drop(reclaim);
        }
        let replacement = Arc::new(Recording::default());
        server
            .replace_native_socket_factory(replacement.clone())
            .await
            .unwrap();
        // Client old channel may await its liveness timeout. Explicitly switch
        // its socket generation too, without changing either Runtime identity.
        client
            .replace_native_socket_factory(client_factory.clone())
            .await
            .unwrap();
        client
            .wait_for_peer("lax", Duration::from_secs(3))
            .await
            .unwrap();
        server
            .wait_for_peer("nyc", Duration::from_secs(3))
            .await
            .unwrap();
        client
            .probe_neighbor("lax", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(server.bound_listeners()[0].address, bound);
        assert_eq!(server.node_id(), "lax");
        assert!(Arc::ptr_eq(
            &old_packet,
            &server.handle.inner.packet_generation()
        ));
        assert!(
            old_packet.retired(),
            "socket replacement must not revive PacketGeneration"
        );
        if transport == TransportKind::NativeTcp {
            assert_eq!(server_factory.listen.load(Ordering::SeqCst), 1);
            assert_eq!(failed.listen.load(Ordering::SeqCst), 1);
            assert_eq!(replacement.listen.load(Ordering::SeqCst), 1);
            assert!(client_factory.dial.load(Ordering::SeqCst) >= 2);
        } else {
            assert_eq!(server_factory.udp.load(Ordering::SeqCst), 1);
            assert_eq!(failed.udp.load(Ordering::SeqCst), 1);
            assert_eq!(replacement.udp.load(Ordering::SeqCst), 1);
            assert_eq!(client_factory.udp.load(Ordering::SeqCst), 2);
        }
        tokio::join!(client.shutdown(), server.shutdown());
        assert!(
            server
                .replace_native_socket_factory(replacement)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn failing_native_dial_never_uses_a_system_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = configs().remove(0);
    config.outgoing.push(OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeTcp,
        endpoint: listener.local_addr().unwrap().to_string(),
        priority: 0,
        id: None,
    });
    let factory = Arc::new(Recording {
        fail: true,
        ..Default::default()
    });
    let runtime = Runtime::start_with_socket_factory(config, factory.clone())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), listener.accept())
            .await
            .is_err()
    );
    assert!(factory.dial.load(Ordering::SeqCst) >= 2);
    assert!(runtime.connected_peers().is_empty());
    runtime.shutdown().await;
}

#[tokio::test]
async fn retained_native_wrappers_cannot_read_or_write_after_socket_retirement() {
    use tokio::io::AsyncReadExt;
    let generation = sockets::SocketGeneration::new();
    let factory = generation.factory(Arc::new(SystemSocketFactory));
    let udp = factory
        .udp(
            "127.0.0.1:0".parse().unwrap(),
            0,
            NativeSocketPurpose::Listener { id: None },
        )
        .unwrap();
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    observer
        .send_to(b"queued", udp.local_addr().unwrap())
        .await
        .unwrap();
    let listener = factory
        .listen(
            "127.0.0.1:0".parse().unwrap(),
            NativeSocketPurpose::Listener { id: None },
        )
        .unwrap();
    let endpoint = listener.local_addr().unwrap();
    let (dialed, accepted) = tokio::join!(
        factory.dial(
            endpoint,
            NativeSocketPurpose::Dial {
                id: None,
                peer: "lax"
            }
        ),
        listener.accept()
    );
    let mut dialed = dialed.unwrap();
    let (mut accepted, _) = accepted.unwrap();
    dialed.write_all(b"queued").await.unwrap();
    let mut bytes = [0; 32];
    let read = accepted.read(&mut bytes);
    tokio::pin!(read);
    generation.retire();
    assert!(read.await.is_err());
    assert!(dialed.write_all(b"must-not-send").await.is_err());
    assert!(listener.accept().await.is_err());
    assert!(udp.recv_from(&mut [0; 32]).await.is_err());
    assert!(
        udp.send_to(b"must-not-send", observer.local_addr().unwrap())
            .await
            .is_err()
    );
    assert!(
        udp.try_send_to(b"must-not-send", observer.local_addr().unwrap())
            .is_err()
    );
    assert!(
        factory
            .udp(
                "127.0.0.1:0".parse().unwrap(),
                0,
                NativeSocketPurpose::Listener { id: None }
            )
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), observer.recv_from(&mut [0; 32]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn retirement_wakes_pending_native_read_and_accept_without_new_network_input() {
    use tokio::io::AsyncReadExt;
    let generation = sockets::SocketGeneration::new();
    let factory = generation.factory(Arc::new(SystemSocketFactory));
    let listener = factory
        .listen(
            "127.0.0.1:0".parse().unwrap(),
            NativeSocketPurpose::Listener { id: None },
        )
        .unwrap();
    let (dialed, accepted) = tokio::join!(
        factory.dial(
            listener.local_addr().unwrap(),
            NativeSocketPurpose::Dial {
                id: None,
                peer: "lax"
            }
        ),
        listener.accept()
    );
    let _dialed = dialed.unwrap();
    let (mut accepted, _) = accepted.unwrap();
    let mut bytes = [0; 32];
    let read = accepted.read(&mut bytes);
    let accept = listener.accept();
    tokio::pin!(read, accept);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut read)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut accept)
            .await
            .is_err()
    );
    generation.retire();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), read)
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), accept)
            .await
            .unwrap()
            .is_err()
    );
}
