use super::*;
use meshway_protocol::stun;

const TIMEOUT: Duration = Duration::from_millis(250);

#[test]
fn noise_token_cookie_collision_does_not_enter_stun_demux() {
    let source = "127.0.0.1:3478".parse().unwrap();
    let mut pending = None;
    let mut datagram = [0_u8; 64];
    // Native UDP Data begins with kind=3 and a per-channel token. A token can
    // contain STUN's cookie by chance; that must not discard every packet on it.
    datagram[0] = 3;
    datagram[4..8].copy_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
    assert!(!crate::runtime::discovery::receive(
        &mut pending,
        &datagram,
        source
    ));
    assert!(crate::runtime::discovery::receive(
        &mut pending,
        &stun::request([7; 12]),
        source,
    ));
}

async fn listener(bind: &str, count: usize) -> Runtime {
    let mut config = configs().remove(0);
    config.limits.work_budget_frames = 2;
    for index in 0..count {
        config.listeners.push(ListenerConfig {
            transport: TransportKind::NativeUdp,
            bind: bind.parse().unwrap(),
            id: Some(format!("listener-{index}")),
            allowed_peers: None,
        });
    }
    Runtime::start(config).await.unwrap()
}

fn mapped(request: &[u8], address: SocketAddr) -> Vec<u8> {
    let mut packet = request[..20].to_vec();
    packet[..2].copy_from_slice(&[1, 1]);
    let mut value = vec![0, if address.is_ipv4() { 1 } else { 2 }];
    value.extend_from_slice(&address.port().to_be_bytes());
    match address.ip() {
        std::net::IpAddr::V4(ip) => value.extend_from_slice(&ip.octets()),
        std::net::IpAddr::V6(ip) => value.extend_from_slice(&ip.octets()),
    }
    packet[2..4].copy_from_slice(&(4_u16 + value.len() as u16).to_be_bytes());
    packet.extend_from_slice(&[0, 1]);
    packet.extend_from_slice(&(value.len() as u16).to_be_bytes());
    packet.extend_from_slice(&value);
    packet
}

async fn request(observer: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut packet = [0; 64];
    let (len, source) =
        tokio::time::timeout(Duration::from_secs(2), observer.recv_from(&mut packet))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(len, 20);
    let transaction: [u8; 12] = packet[8..20].try_into().unwrap();
    assert_eq!(packet[..len], stun::request(transaction));
    (packet[..len].to_vec(), source)
}

async fn observe(runtime: &Runtime, observer: &UdpSocket, info: &UdpSocketInfo) {
    observe_with_replies(runtime, observer, info, 1).await;
}

async fn observe_with_replies(
    runtime: &Runtime,
    observer: &UdpSocket,
    info: &UdpSocketInfo,
    replies: usize,
) {
    let discovery = runtime.discover_udp(info.socket_id, observer.local_addr().unwrap(), TIMEOUT);
    let reply = async {
        let (packet, source) = request(observer).await;
        assert_eq!(source.port(), info.local_addr.port());
        if !info.local_addr.ip().is_unspecified() {
            assert_eq!(source, info.local_addr);
        }
        for reply in 0..replies {
            if reply != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            observer
                .send_to(&mapped(&packet, source), source)
                .await
                .unwrap();
        }
        source
    };
    let (observation, source) = tokio::join!(discovery, reply);
    let observation = observation.unwrap();
    assert_eq!(observation.socket, *info);
    assert_eq!(observation.observer, observer.local_addr().unwrap());
    assert_eq!(observation.outcome, UdpDiscoveryOutcome::Mapped);
    assert_eq!(observation.mapped_endpoint, Some(source));
    assert!(observation.round_trip.unwrap() < TIMEOUT);
}

#[tokio::test]
async fn discovery_during_noise_handshake_and_neighbor_probe() {
    let mut configs = configs();
    configs[1].listeners.push(ListenerConfig {
        transport: TransportKind::NativeUdp,
        bind: "127.0.0.1:0".parse().unwrap(),
        id: Some("listener".into()),
        allowed_peers: None,
    });
    let acceptor = Runtime::start(configs[1].clone()).await.unwrap();
    configs[0].outgoing.push(OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeUdp,
        endpoint: acceptor.bound_listeners()[0].address.to_string(),
        priority: 0,
        id: Some("candidate".into()),
    });
    let dialer = Runtime::start(configs[0].clone()).await.unwrap();
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let discovery = async {
        for _ in 0..4 {
            observe(&dialer, &observer, &dialer.udp_sockets()[0]).await;
            observe(&acceptor, &observer, &acceptor.udp_sockets()[0]).await;
        }
    };
    let noise = async {
        dialer
            .wait_for_peer("lax", Duration::from_secs(2))
            .await
            .unwrap();
        acceptor
            .wait_for_peer("nyc", Duration::from_secs(2))
            .await
            .unwrap();
        for _ in 0..4 {
            dialer
                .probe_neighbor("lax", Duration::from_secs(1))
                .await
                .unwrap();
            acceptor
                .probe_neighbor("nyc", Duration::from_secs(1))
                .await
                .unwrap();
        }
    };
    tokio::join!(discovery, noise);
    tokio::join!(dialer.shutdown(), acceptor.shutdown());
}

#[tokio::test]
async fn discovery_queued_cancel_sends_nothing_and_retired_success_is_closed() {
    let runtime = listener("127.0.0.1:0", 1).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = observer.local_addr().unwrap();
    {
        let mut call = Box::pin(runtime.discover_udp(0, address, TIMEOUT));
        let poll = std::future::poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
        assert!(poll.is_pending());
        // No engine poll has occurred after enqueue; cancelling must prevent send.
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), observer.recv_from(&mut [0; 64]))
            .await
            .is_err()
    );
    assert_eq!(
        runtime.handle.inner.udp_discovery_slots.available_permits(),
        16
    );
    let mut call = Box::pin(runtime.discover_udp(0, address, TIMEOUT));
    let poll = std::future::poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    assert!(poll.is_pending());
    let (packet, source) = request(&observer).await;
    observer
        .send_to(&mapped(&packet, source), source)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_millis(100), async {
        while runtime.handle.inner.udp_discovery_slots.available_permits() != 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // A valid observation is queued, but this generation closes before caller resumes.
    runtime.shutdown().await;
    assert_eq!(call.await.unwrap_err(), Error::Closed);
}

#[tokio::test]
async fn discovery_engine_exit_removes_socket_and_closes_pending() {
    let runtime = listener("127.0.0.1:0", 1).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let info = runtime.udp_sockets()[0].clone();
    let address = observer.local_addr().unwrap();
    let clone = runtime.clone();
    let call = tokio::spawn(async move {
        clone
            .discover_udp(0, address, Duration::from_millis(1500))
            .await
    });
    request(&observer).await;
    // Dispatch is first; this Runtime has exactly one UDP engine afterward.
    lock(&runtime.handle.inner.handles)[1].abort();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        Error::Closed
    );
    assert!(runtime.udp_sockets().is_empty());
    assert!(std::net::UdpSocket::bind(info.local_addr).is_ok());
    assert_eq!(
        runtime.handle.inner.udp_discovery_slots.available_permits(),
        16
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn discovery_real_listener_and_candidate_ports_with_noise_traffic() {
    let fixture = Fixture::native(TransportKind::NativeUdp).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let sockets = [
        fixture.lax.udp_sockets(),
        fixture.nyc.udp_sockets(),
        fixture.sha.udp_sockets(),
    ];
    assert!(sockets.iter().all(|sockets| sockets.len() == 1));
    let measurements = async {
        for _ in 0..4 {
            for (runtime, sockets) in [
                (&fixture.lax, &sockets[0]),
                (&fixture.nyc, &sockets[1]),
                (&fixture.sha, &sockets[2]),
            ] {
                observe(runtime, &observer, &sockets[0]).await;
            }
        }
    };
    tokio::join!(measurements, fixture.exercise(TransportKind::NativeUdp));
    fixture
        .nyc
        .probe_neighbor("lax", Duration::from_secs(1))
        .await
        .unwrap();
    fixture.shutdown().await;
}

#[tokio::test]
async fn discovery_metadata_ipv6_and_replace() {
    let runtime = listener("127.0.0.1:0", 2).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let infos = runtime.udp_sockets();
    assert_ne!(infos[0].socket_id, infos[1].socket_id);
    for info in &infos {
        assert_eq!(
            info.listener_id,
            Some(format!("listener-{}", info.socket_id))
        );
        assert_eq!(info.candidate_id, None);
        observe(&runtime, &observer, info).await;
    }
    let clone = runtime.clone();
    runtime.shutdown().await;
    assert!(clone.udp_sockets().is_empty());
    for info in infos {
        assert!(std::net::UdpSocket::bind(info.local_addr).is_ok());
    }
    let replacement = listener("127.0.0.1:0", 1).await;
    observe(&replacement, &observer, &replacement.udp_sockets()[0]).await;
    replacement.shutdown().await;
    // IPv6 is exercised when the host supports loopback IPv6.
    if let Ok(observer) = UdpSocket::bind("[::1]:0").await {
        let runtime = listener("[::1]:0", 1).await;
        observe(&runtime, &observer, &runtime.udp_sockets()[0]).await;
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn discovery_bad_transaction_source_and_codec_do_not_poison_valid_response() {
    let runtime = listener("127.0.0.1:0", 1).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let attacker = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let info = runtime.udp_sockets()[0].clone();
    let discovery = runtime.discover_udp(info.socket_id, observer.local_addr().unwrap(), TIMEOUT);
    let responses = async {
        let (packet, source) = request(&observer).await;
        let valid = mapped(&packet, source);
        attacker.send_to(&valid, source).await.unwrap();
        let mut wrong = valid.clone();
        wrong[8] ^= 1;
        observer.send_to(&wrong, source).await.unwrap();
        let mut bad = valid.clone();
        bad[3] = 0;
        observer.send_to(&bad, source).await.unwrap();
        observer.send_to(&packet, source).await.unwrap(); // Binding request, never reflected.
        let mut oversized = valid.clone();
        oversized.resize(1200, 0);
        observer.send_to(&oversized, source).await.unwrap();
        observer.send_to(&valid, source).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), observer.recv_from(&mut [0; 64]))
                .await
                .is_err()
        );
    };
    let (observation, ()) = tokio::join!(discovery, responses);
    assert_eq!(observation.unwrap().outcome, UdpDiscoveryOutcome::Mapped);
    assert!(runtime.connected_peers().is_empty());
    assert_eq!(runtime.statistics().native_udp.received, 0);
    runtime.shutdown().await;
}

#[tokio::test]
async fn discovery_invalid_stun_error_and_timeout_are_bounded() {
    let runtime = listener("127.0.0.1:0", 1).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let info = runtime.udp_sockets()[0].clone();
    for outcome in [
        UdpDiscoveryOutcome::Timeout,
        UdpDiscoveryOutcome::InvalidResponse,
        UdpDiscoveryOutcome::StunError,
    ] {
        let started = Instant::now();
        let discovery =
            runtime.discover_udp(info.socket_id, observer.local_addr().unwrap(), TIMEOUT);
        let response = async {
            let (packet, source) = request(&observer).await;
            match outcome {
                UdpDiscoveryOutcome::InvalidResponse => {
                    observer
                        .send_to(&mapped(&packet, "127.0.0.1:0".parse().unwrap()), source)
                        .await
                        .unwrap();
                }
                UdpDiscoveryOutcome::StunError => {
                    let mut error = packet;
                    error[..4].copy_from_slice(&[1, 0x11, 0, 8]);
                    error.extend_from_slice(&[0, 9, 0, 4, 0, 0, 4, 0]);
                    observer.send_to(&error, source).await.unwrap();
                }
                _ => (),
            }
        };
        let (result, ()) = tokio::join!(discovery, response);
        let result = result.unwrap();
        assert_eq!(result.outcome, outcome);
        assert!(result.mapped_endpoint.is_none() && result.round_trip.is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
        tokio::task::yield_now().await;
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn discovery_cancel_shutdown_drop_and_capacity_release() {
    let runtime = listener("127.0.0.1:0", 1).await;
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = observer.local_addr().unwrap();
    let info = runtime.udp_sockets()[0].clone();
    let clone = runtime.clone();
    let task = tokio::spawn(async move {
        clone
            .discover_udp(info.socket_id, address, Duration::from_millis(1500))
            .await
    });
    request(&observer).await;
    assert_eq!(
        runtime.discover_udp(0, address, TIMEOUT).await.unwrap_err(),
        Error::Capacity
    );
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_millis(100), async {
        while runtime.handle.inner.udp_discovery_slots.available_permits() != 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    observe(&runtime, &observer, &runtime.udp_sockets()[0]).await;
    let clone = runtime.clone();
    let task = tokio::spawn(async move {
        clone
            .discover_udp(0, address, Duration::from_millis(1500))
            .await
    });
    request(&observer).await;
    runtime.shutdown().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        Error::Closed
    );
    assert_eq!(
        runtime.handle.inner.udp_discovery_slots.available_permits(),
        16
    );
    // Last public handle drop stops the engine without explicit shutdown.
    let runtime = listener("127.0.0.1:0", 1).await;
    let address = runtime.udp_sockets()[0].local_addr;
    let inner = runtime.handle.inner.clone();
    drop(runtime);
    tokio::time::timeout(Duration::from_millis(200), async {
        while !lock(&inner.udp_sockets).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(std::net::UdpSocket::bind(address).is_ok());
}

#[tokio::test]
async fn discovery_global_cap_validation_and_flood_deadline() {
    let mut config = configs().remove(0);
    config.limits.work_budget_frames = 2;
    let mut targets = Vec::new();
    for index in 0..17 {
        let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        config.outgoing.push(OutgoingCandidate {
            peer: "lax".into(),
            transport: TransportKind::NativeUdp,
            endpoint: target.local_addr().unwrap().to_string(),
            priority: index,
            id: Some(format!("candidate-{index}")),
        });
        targets.push(target);
    }
    let runtime = Runtime::start(config).await.unwrap();
    assert!(
        runtime
            .udp_sockets()
            .iter()
            .all(|info| info.listener_id.is_none()
                && info.candidate_id == Some(format!("candidate-{}", info.socket_id)))
    );
    let observer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = observer.local_addr().unwrap();
    for invalid in [
        "127.0.0.1:0",
        "0.0.0.0:1234",
        "224.0.0.1:1234",
        "255.255.255.255:1234",
        "[::1]:1234",
        "[::ffff:127.0.0.1]:1234",
        "[fe80::1%1]:1234",
    ] {
        assert!(matches!(
            runtime
                .discover_udp(0, invalid.parse().unwrap(), TIMEOUT)
                .await,
            Err(Error::InvalidConfig(_))
        ));
    }
    for timeout in [
        Duration::from_millis(249),
        Duration::from_millis(1501),
        Duration::MAX,
    ] {
        assert!(matches!(
            runtime.discover_udp(0, address, timeout).await,
            Err(Error::InvalidConfig(_))
        ));
    }
    let mut tasks = Vec::new();
    for id in 0..16 {
        let clone = runtime.clone();
        tasks.push(tokio::spawn(async move {
            clone
                .discover_udp(id, address, Duration::from_millis(1500))
                .await
        }));
        request(&observer).await;
    }
    assert_eq!(
        runtime
            .discover_udp(16, address, TIMEOUT)
            .await
            .unwrap_err(),
        Error::Capacity
    );
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
    tokio::time::timeout(Duration::from_millis(100), async {
        while runtime.handle.inner.udp_discovery_slots.available_permits() != 16 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let info = runtime.udp_sockets()[0].clone();
    let discovery = runtime.discover_udp(0, address, TIMEOUT);
    let flood = async {
        let (packet, source) = request(&observer).await;
        let mut bad = mapped(&packet, source);
        bad[3] = 0;
        let until = Instant::now() + Duration::from_millis(350);
        while Instant::now() < until {
            for _ in 0..32 {
                observer.send_to(&bad, source).await.unwrap();
            }
            tokio::task::yield_now().await;
        }
    };
    let started = Instant::now();
    let (result, ()) = tokio::join!(discovery, flood);
    assert_eq!(
        result.unwrap().outcome,
        UdpDiscoveryOutcome::InvalidResponse
    );
    assert!(started.elapsed() < Duration::from_millis(600));
    assert!(runtime.connected_peers().is_empty());
    // A saturated UDP receive buffer can drop the first recovery reply even
    // after the flood producer exits. Retry only this post-flood recovery probe;
    // all transaction/source/codec checks and the flood deadline remain strict.
    observe_with_replies(&runtime, &observer, &info, 4).await;
    runtime.shutdown().await;
}
