//! Dataplane primitive tests. Controller scheduling and real NAT/TUN acceptance
//! are deliberately separate integration gates, documented in the handoff.
use super::*;

fn endpoint(socket: &UdpPunchSocket) -> SocketAddr {
    if socket.info.local_addr.is_ipv4() {
        SocketAddr::from(([127, 0, 0, 1], socket.info.local_addr.port()))
    } else {
        SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], socket.info.local_addr.port()))
    }
}

async fn pair(ipv6: bool) -> (Runtime, Runtime, UdpPunchSocket, UdpPunchSocket) {
    let mut config = configs();
    // Authorize these exact endpoint paths; the punch API itself adds none.
    for config in &mut config[..2] {
        config.allowed_paths.extend([
            vec!["nyc".into(), "lax".into()],
            vec!["lax".into(), "nyc".into()],
        ]);
    }
    let a = Runtime::start(config.remove(0)).await.unwrap();
    let b = Runtime::start(config.remove(0)).await.unwrap();
    let sa = a
        .open_udp_punch_socket("lax", "punch-lax", ipv6)
        .await
        .unwrap();
    let sb = b
        .open_udp_punch_socket("nyc", "punch-nyc", ipv6)
        .await
        .unwrap();
    (a, b, sa, sb)
}

async fn punch_pair(a: &Runtime, b: &Runtime, sa: &UdpPunchSocket, sb: &UdpPunchSocket) {
    let (oa, ob) = tokio::join!(
        a.punch_udp(sa, vec![endpoint(sb)], 0, Duration::from_secs(2)),
        b.punch_udp(sb, vec![endpoint(sa)], 0, Duration::from_secs(2)),
    );
    for (observation, socket, remote, peer) in [
        (oa.unwrap(), sa, endpoint(sb), "lax"),
        (ob.unwrap(), sb, endpoint(sa), "nyc"),
    ] {
        assert_eq!(observation.socket, socket.info);
        assert_eq!(observation.connection.transport, TransportKind::NativeUdp);
        assert_eq!(observation.connection.remote_addr, Some(remote));
        assert_eq!(
            observation.connection.candidate_id,
            socket.info.candidate_id
        );
        assert_eq!(observation.connection.listener_id, None);
        assert_eq!(observation.connection.peer_node_id, peer);
    }
    // Packet exchange proves the key-confirmed channel carries existing E2E,
    // not just that two addresses appeared in a STUN observation.
    for (runtime, target) in [(a, "lax"), (b, "nyc")] {
        let path = vec![runtime.node_id().to_owned(), target.into()];
        let reply = runtime
            .probe(path.clone(), b"punch-e2e".to_vec())
            .await
            .unwrap();
        assert_eq!(reply.path, path);
        assert_eq!(reply.payload, b"punch-e2e");
    }
}

#[tokio::test]
async fn outbound_sockets_stun_noise_and_e2e_share_exact_ports() {
    let (a, b, sa, sb) = pair(false).await;
    assert!(a.bound_listeners().is_empty() && b.bound_listeners().is_empty());
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    // Existing STUN demux is exercised on the new actual carrier socket.
    for (runtime, socket) in [(&a, &sa), (&b, &sb)] {
        let measurement = runtime.discover_udp(
            socket.info.socket_id,
            observer.local_addr().unwrap(),
            Duration::from_millis(500),
        );
        let reflect = async {
            let mut packet = [0; 64];
            let (len, source) = observer.recv_from(&mut packet).await.unwrap();
            assert_eq!(source, endpoint(socket));
            let mut response = packet[..len].to_vec();
            response[..4].copy_from_slice(&[1, 1, 0, 12]);
            response.extend_from_slice(&[0, 1, 0, 8, 0, 1]);
            response.extend_from_slice(&source.port().to_be_bytes());
            response.extend_from_slice(&[127, 0, 0, 1]);
            observer.send_to(&response, source).await.unwrap();
        };
        let (result, ()) = tokio::join!(measurement, reflect);
        assert_eq!(result.unwrap().mapped_endpoint, Some(endpoint(socket)));
    }
    punch_pair(&a, &b, &sa, &sb).await;
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn outbound_socket_is_not_an_implicit_listener() {
    let (a, b, sa, sb) = pair(false).await;
    // Only one side receives coordination. A normal authenticated IK Hello to
    // the other outbound socket must still be rejected outside its window.
    assert_eq!(
        b.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_millis(600))
            .await
            .unwrap_err(),
        Error::Timeout
    );
    assert!(a.connected_peers().is_empty() && b.connected_peers().is_empty());
    // Caller-side deadline cancels asynchronously; capacity remains held until
    // the engine has actually discarded its pending authentication state.
    tokio::time::timeout(Duration::from_millis(250), async {
        while b.handle.inner.udp_punch_slots.available_permits() != 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    punch_pair(&a, &b, &sa, &sb).await;
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn candidate_rotation_can_skip_an_unreachable_hint() {
    let (a, b, sa, sb) = pair(false).await;
    let dead = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (oa, ob) = tokio::join!(
        a.punch_udp(&sa, vec![endpoint(&sb)], 0, Duration::from_secs(3)),
        b.punch_udp(
            &sb,
            vec![dead.local_addr().unwrap(), endpoint(&sa)],
            0,
            Duration::from_secs(3)
        ),
    );
    assert_eq!(oa.unwrap().connection.remote_addr, Some(endpoint(&sb)));
    assert_eq!(ob.unwrap().connection.remote_addr, Some(endpoint(&sa)));
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn punch_does_not_add_access_or_bypass_a_mandatory_hop() {
    let config = configs();
    // These configs authorize nyc -> lax -> sha only, never nyc -> sha.
    let a = Runtime::start(config[0].clone()).await.unwrap();
    let b = Runtime::start(config[2].clone()).await.unwrap();
    let sa = a
        .open_udp_punch_socket("sha", "punch-sha", false)
        .await
        .unwrap();
    let sb = b
        .open_udp_punch_socket("nyc", "punch-nyc", false)
        .await
        .unwrap();
    let (oa, ob) = tokio::join!(
        a.punch_udp(&sa, vec![endpoint(&sb)], 0, Duration::from_secs(2)),
        b.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_secs(2)),
    );
    oa.unwrap();
    ob.unwrap();
    assert_eq!(
        a.probe(vec!["nyc".into(), "sha".into()], vec![1])
            .await
            .unwrap_err(),
        Error::UnauthorizedPath
    );
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn cancellation_deadline_capacity_and_generation_are_bounded() {
    let (a, b, sa, sb) = pair(false).await;
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let clone = b.clone();
    let socket = sb.clone();
    let target = sink.local_addr().unwrap();
    let call = tokio::spawn(async move {
        clone
            .punch_udp(&socket, vec![target], 0, Duration::from_secs(5))
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), sink.recv_from(&mut [0; 512]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        b.punch_udp(&sb, vec![target], 0, Duration::from_secs(1))
            .await
            .unwrap_err(),
        Error::Capacity
    );
    call.abort();
    let _ = call.await;
    tokio::time::timeout(Duration::from_millis(250), async {
        while b.handle.inner.udp_punch_slots.available_permits() != 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    punch_pair(&a, &b, &sa, &sb).await;
    b.replace_native_socket_factory(Arc::new(sockets::SystemSocketFactory))
        .await
        .unwrap();
    assert_eq!(
        b.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_secs(1))
            .await
            .unwrap_err(),
        Error::Closed
    );
    assert!(b.udp_sockets().is_empty());
    assert_eq!(
        a.punch_udp(&sb, vec![endpoint(&sa)], 0, Duration::from_secs(1))
            .await
            .unwrap_err(),
        Error::Unavailable
    );
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn punch_ipv6_uses_the_same_authentication_boundary() {
    if UdpSocket::bind("[::1]:0").await.is_err() {
        return;
    }
    let (a, b, sa, sb) = pair(true).await;
    punch_pair(&a, &b, &sa, &sb).await;
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn invalid_hints_and_unauthorized_peer_create_no_socket_or_attempt() {
    let mut config = configs().remove(0);
    config
        .trusted_peers
        .iter_mut()
        .find(|peer| peer.node_id == "sha")
        .unwrap()
        .transports = vec![TransportKind::Vless];
    let a = Runtime::start(config).await.unwrap();
    assert!(matches!(
        a.open_udp_punch_socket("sha", "denied", false).await,
        Err(Error::UnauthorizedTransport)
    ));
    assert!(a.udp_sockets().is_empty());
    let sa = a
        .open_udp_punch_socket("lax", "punch", false)
        .await
        .unwrap();
    assert!(matches!(
        a.open_udp_punch_socket("lax", "punch", false).await,
        Err(Error::InvalidConfig(_))
    ));
    for invalid in [
        "0.0.0.0:2345",
        "127.0.0.1:0",
        "224.0.0.1:2345",
        "[::1]:2345",
        "255.255.255.255:2345",
    ] {
        assert!(matches!(
            a.punch_udp(
                &sa,
                vec![invalid.parse().unwrap()],
                0,
                Duration::from_secs(1)
            )
            .await,
            Err(Error::InvalidConfig(_))
        ));
    }
    let target = "127.0.0.1:12345".parse().unwrap();
    for candidates in [vec![], vec![target; 2], vec![target; 9]] {
        assert!(matches!(
            a.punch_udp(&sa, candidates, 0, Duration::from_secs(1))
                .await,
            Err(Error::InvalidConfig(_))
        ));
    }
    a.shutdown().await;
}

#[tokio::test]
async fn candidate_address_is_not_peer_identity_or_admission() {
    // A cryptographically trusted but different peer exercises scoped
    // admission, rather than only unknown-key rejection.
    let config = configs();
    let a = Runtime::start(config[0].clone()).await.unwrap();
    let sa = a
        .open_udp_punch_socket("lax", "expected-lax", false)
        .await
        .unwrap();
    let mut attacker_config = config[2].clone();
    attacker_config.outgoing.push(OutgoingCandidate {
        peer: "nyc".into(),
        transport: TransportKind::NativeUdp,
        endpoint: endpoint(&sa).to_string(),
        priority: 0,
        id: Some("attacker".into()),
    });
    let attacker = Runtime::start(attacker_config).await.unwrap();
    let mut attacker_address = attacker.udp_sockets()[0].local_addr;
    attacker_address.set_ip("127.0.0.1".parse().unwrap());
    assert_eq!(
        a.punch_udp(&sa, vec![attacker_address], 0, Duration::from_millis(600))
            .await
            .unwrap_err(),
        Error::Timeout
    );
    assert!(a.connected_peers().is_empty() && attacker.connected_peers().is_empty());
    assert!(a.statistics().native_udp.drop_reasons.unauthorized_carrier > 0);
    tokio::join!(a.shutdown(), attacker.shutdown());
}

#[tokio::test]
async fn blocked_udp_preserves_stream_relay_and_e2e_path() {
    let fixture = Fixture::native(TransportKind::NativeTcp).await;
    let sa = fixture
        .nyc
        .open_udp_punch_socket("sha", "punch-sha", false)
        .await
        .unwrap();
    let sb = fixture
        .sha
        .open_udp_punch_socket("nyc", "punch-nyc", false)
        .await
        .unwrap();
    let sinks = [
        UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        UdpSocket::bind("127.0.0.1:0").await.unwrap(),
    ];
    let (oa, ob, ()) = tokio::join!(
        fixture.nyc.punch_udp(
            &sa,
            vec![sinks[0].local_addr().unwrap()],
            0,
            Duration::from_millis(600)
        ),
        fixture.sha.punch_udp(
            &sb,
            vec![sinks[1].local_addr().unwrap()],
            0,
            Duration::from_millis(600)
        ),
        fixture.exercise(TransportKind::NativeTcp),
    );
    assert_eq!(oa.unwrap_err(), Error::Timeout);
    assert_eq!(ob.unwrap_err(), Error::Timeout);
    assert!(!fixture.nyc.connected_peers().contains(&"sha".into()));
    assert!(!fixture.sha.connected_peers().contains(&"nyc".into()));
    assert!(
        fixture
            .nyc
            .peer_connections()
            .iter()
            .all(|connection| connection.transport == TransportKind::NativeTcp)
    );
    fixture.exercise(TransportKind::NativeTcp).await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn retiring_socket_owner_reclaims_live_budget_without_reusing_ordinals() {
    let a = Runtime::start(configs().remove(0)).await.unwrap();
    let mut last = None;
    for _ in 0..32 {
        let socket = a
            .open_udp_punch_socket("lax", "owned-lax", false)
            .await
            .unwrap();
        if let Some(previous) = last {
            assert!(socket.info.socket_id > previous);
        }
        last = Some(socket.info.socket_id);
        assert!(matches!(
            a.open_udp_punch_socket("lax", "other-lax", false).await,
            Err(Error::Capacity)
        ));
        let other = a
            .open_udp_punch_socket("sha", "owned-sha", false)
            .await
            .unwrap();
        assert_eq!(
            a.handle.native.lock().await.punch_slots.available_permits(),
            14
        );
        a.close_udp_punch_socket(&socket).await.unwrap();
        a.close_udp_punch_socket(&other).await.unwrap();
        // Capability clones cannot keep the actual fd/permit alive after close.
        assert!(a.udp_sockets().is_empty());
        assert_eq!(
            a.handle.native.lock().await.punch_slots.available_permits(),
            16
        );
        assert!(a.handle.native.lock().await.tasks.is_empty());
        assert_eq!(
            a.punch_udp(
                &socket,
                vec!["127.0.0.1:12345".parse().unwrap()],
                0,
                Duration::from_secs(1)
            )
            .await
            .unwrap_err(),
            Error::Closed
        );
    }
    a.shutdown().await;
}

type PendingPunch<'a> = Pin<Box<dyn Future<Output = Result<UdpPunchObservation>> + 'a>>;

async fn queued_pair<'a>(
    a: &'a Runtime,
    b: &'a Runtime,
    sa: &'a UdpPunchSocket,
    sb: &'a UdpPunchSocket,
    priority: u16,
) -> (PendingPunch<'a>, PendingPunch<'a>) {
    let mut ca = Box::pin(a.punch_udp(sa, vec![endpoint(sb)], priority, Duration::from_secs(2)));
    let mut cb = Box::pin(b.punch_udp(sb, vec![endpoint(sa)], priority, Duration::from_secs(2)));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(ca.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(cb.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while a.handle.inner.udp_punch_slots.available_permits() != 16
            || b.handle.inner.udp_punch_slots.available_permits() != 16
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    (ca, cb)
}

#[tokio::test]
async fn queued_receipt_cannot_borrow_a_better_channel_and_close_keeps_its_owner() {
    let (a, b, sa, sb) = pair(false).await;
    let (ca, cb) = queued_pair(&a, &b, &sa, &sb, 7).await;
    let (left, right) = tokio::io::duplex(64 * 1024);
    let (oa, ob) = tokio::join!(
        a.attach_stream_with_priority(
            left,
            StreamRole::Dialer { peer: "lax".into() },
            TransportKind::NativeTcp,
            0
        ),
        b.attach_stream_with_priority(right, StreamRole::Acceptor, TransportKind::NativeTcp, 0),
    );
    oa.unwrap();
    ob.unwrap();
    assert_eq!(ca.await.unwrap_err(), Error::Unavailable);
    assert_eq!(cb.await.unwrap_err(), Error::Unavailable);
    a.close_udp_punch_socket(&sa).await.unwrap();
    b.close_udp_punch_socket(&sb).await.unwrap();
    assert_eq!(a.peer_transport("lax"), Some(TransportKind::NativeTcp));
    assert_eq!(b.peer_transport("nyc"), Some(TransportKind::NativeTcp));
    a.probe_neighbor("lax", Duration::from_secs(1))
        .await
        .unwrap();
    tokio::join!(a.shutdown(), b.shutdown());
}

#[tokio::test]
async fn cancelling_a_committed_attempt_preserves_the_authenticated_session() {
    let (a, b, sa, sb) = pair(false).await;
    let (ca, cb) = queued_pair(&a, &b, &sa, &sb, 0).await;
    drop(ca);
    drop(cb);
    // Cancellation retires only uncommitted attempt work; fresh key-confirmed
    // sessions already published before cancellation keep normal liveness.
    a.probe_neighbor("lax", Duration::from_secs(1))
        .await
        .unwrap();
    b.probe_neighbor("nyc", Duration::from_secs(1))
        .await
        .unwrap();
    a.close_udp_punch_socket(&sa).await.unwrap();
    assert!(!a.connected_peers().contains(&"lax".into()));
    tokio::join!(a.shutdown(), b.shutdown());
}
