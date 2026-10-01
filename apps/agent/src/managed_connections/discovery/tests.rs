use super::*;
use crate::managed_connections::{
    Manager, Reason,
    tests::{fixture, snapshot},
};
use meshway_protocol::ConnectionTransport;

#[test]
fn injected_clock_regression_suppresses_samples_across_generations_without_clamping() {
    let timestamps = Arc::new(TimestampGate::default());
    let original = DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let readings = [
        original,
        original - chrono::Duration::seconds(10),
        original,
        original + chrono::Duration::nanoseconds(1),
    ];
    let mut clock = readings.into_iter();
    assert_eq!(timestamps.observe(clock.next().unwrap()), Some(original));
    let next_generation = timestamps.clone();
    assert_eq!(next_generation.observe(clock.next().unwrap()), None);
    assert_eq!(next_generation.observe(clock.next().unwrap()), None);
    let genuinely_later = clock.next().unwrap();
    assert_eq!(
        next_generation.observe(genuinely_later),
        Some(genuinely_later)
    );
    // No synthetic timestamp can be returned during rollback or equality.
    assert_eq!(timestamps.observe(original), None);
}

async fn observer() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Receiver<std::net::SocketAddr>,
) {
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = udp.local_addr().unwrap();
    let (send, receive) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(async move {
        let mut bytes = [0_u8; 2048];
        loop {
            let (len, source) = udp.recv_from(&mut bytes).await.unwrap();
            if len != 20 || bytes[..2] != [0, 1] {
                continue;
            }
            send.send(source).await.unwrap();
            // RFC 8489 XOR-MAPPED-ADDRESS; the observer sees the real carrier port.
            let mut response = bytes[..20].to_vec();
            response[..4].copy_from_slice(&[1, 1, 0, 12]);
            response.extend_from_slice(&[0, 0x20, 0, 8, 0, 1]);
            response.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
            let std::net::IpAddr::V4(ip) = source.ip() else {
                panic!("IPv4 fixture")
            };
            response.extend(
                ip.octets()
                    .iter()
                    .zip([0x21, 0x12, 0xa4, 0x42])
                    .map(|(a, b)| a ^ b),
            );
            udp.send_to(&response, source).await.unwrap();
        }
    });
    (address, task, receive)
}

async fn mapped(manager: &Manager) -> Vec<UdpSocketDiscovery> {
    time::timeout(Duration::from_secs(4), async {
        loop {
            let (state, sockets, _) = manager.discovery_snapshot();
            if state == State::Observed {
                return sockets;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn actual_listener_and_outgoing_ports_coexist_with_authenticated_noise() {
    let (observer, task, mut sources) = observer().await;
    let (server, skey, client, ckey) = fixture(ConnectionTransport::NativeUdp);
    let options = crate::EndpointDiscoveryOptions::new(vec![observer]).unwrap();
    let mut a = Manager::new(None, Duration::from_secs(10));
    let mut b = Manager::new(None, Duration::from_secs(10));
    a.enable_discovery(options.clone());
    b.enable_discovery(options);
    a.reconcile(snapshot(server.clone()), skey, Instant::now())
        .await;
    b.reconcile(snapshot(client.clone()), ckey, Instant::now())
        .await;
    let left = mapped(&a).await;
    let right = mapped(&b).await;
    assert_eq!(left[0].bound_address, server.listeners[0].bind);
    assert_eq!(left[0].resource_id, server.listeners[0].id);
    assert_eq!(right[0].resource_id, client.outgoing[0].id);
    assert_eq!(
        right[0].resource_kind,
        UdpDiscoveryResourceKind::OutgoingCandidate
    );
    let mut seen = Vec::new();
    time::timeout(Duration::from_secs(2), async {
        while !left
            .iter()
            .chain(&right)
            .filter(|socket| !socket.observations.is_empty())
            .all(|socket| seen.contains(socket.observations[0].mapped_endpoint.as_ref().unwrap()))
        {
            seen.push(sources.recv().await.unwrap().to_string());
        }
    })
    .await
    .unwrap();
    for socket in left
        .iter()
        .chain(&right)
        .filter(|socket| !socket.observations.is_empty())
    {
        assert!(seen.contains(socket.observations[0].mapped_endpoint.as_ref().unwrap()));
        let mapped: std::net::SocketAddr = socket.observations[0]
            .mapped_endpoint
            .as_ref()
            .unwrap()
            .parse()
            .unwrap();
        let bound: std::net::SocketAddr = socket.bound_address.parse().unwrap();
        assert_eq!(mapped.port(), bound.port());
        assert_ne!(bound.port(), 0);
    }
    time::timeout(Duration::from_secs(8), async {
        loop {
            if a.observation()
                .peers
                .iter()
                .chain(&b.observation().peers)
                .any(|p| p.probe_fresh)
            {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    a.withdraw(Reason::ShuttingDown).await;
    b.withdraw(Reason::ShuttingDown).await;
    assert_eq!(a.discovery_snapshot().0, State::Withdrawn);
    assert!(a.discovery_snapshot().1.is_empty());
    task.abort();
}

#[tokio::test]
async fn renewal_preserves_expiry_revision_and_shutdown_fence_pending_reports() {
    let (observer, task, _sources) = observer().await;
    let (mut intent, key, _, _) = fixture(ConnectionTransport::NativeUdp);
    let mut manager = Manager::new(None, Duration::from_secs(10));
    manager.enable_discovery(crate::EndpointDiscoveryOptions::new(vec![observer]).unwrap());
    manager
        .reconcile(snapshot(intent.clone()), key.clone(), Instant::now())
        .await;
    let original = mapped(&manager).await;
    let (_, _, mut old_guard) = manager.discovery_snapshot();
    manager
        .reconcile(snapshot(intent.clone()), key.clone(), Instant::now())
        .await;
    let current = mapped(&manager).await;
    assert_eq!(original[0].generation, current[0].generation);
    assert_eq!(
        original[0].observations[0].expires_at,
        current[0].observations[0].expires_at
    );
    assert!(!old_guard.as_ref().unwrap().current());
    intent.revision += 1;
    manager.invalidate_if_changed(&snapshot(intent.clone()));
    assert_eq!(manager.discovery_snapshot().0, State::Withdrawn);
    time::timeout(
        Duration::from_millis(100),
        old_guard.as_mut().unwrap().changed(),
    )
    .await
    .unwrap();
    manager
        .reconcile(snapshot(intent), key, Instant::now())
        .await;
    let next = mapped(&manager).await;
    assert!(next[0].generation > original[0].generation);
    let (_, _, guard) = manager.discovery_snapshot();
    manager.withdraw(Reason::ShuttingDown).await;
    assert!(!guard.unwrap().current());
    task.abort();
}

#[tokio::test]
async fn expiry_prunes_without_utc_clock_or_lease_extension() {
    let now = Instant::now();
    let utc = Utc::now();
    let mut publication = Publication::new(
        Source {
            revision: 1,
            digest: "a".repeat(64),
        },
        State::Observed,
    );
    publication.sockets.push(UdpSocketDiscovery {
        resource_kind: UdpDiscoveryResourceKind::Listener,
        resource_id: "listener".into(),
        generation: 1,
        socket_id: 1,
        bound_address: "127.0.0.1:1234".into(),
        source: EndpointDiscoverySource::ManagedUdpSocketStun,
        observations: vec![StunEndpointObservation {
            observer: "127.0.0.1:5678".into(),
            outcome: StunDiscoveryOutcome::Mapped,
            mapped_endpoint: Some("127.0.0.1:1234".into()),
            rtt_ms: Some(0),
            observed_at: utc,
            expires_at: utc + chrono::Duration::seconds(30),
        }],
    });
    publication.expiries.push((1, "127.0.0.1:5678".into(), now));
    publication.prune();
    assert!(publication.sockets[0].observations.is_empty());
    assert_eq!(publication.state, State::Probing);
    assert_eq!(
        absolute_expiry(utc, now, now + Duration::from_secs(5)),
        utc + chrono::Duration::seconds(5)
    );
}

#[tokio::test]
async fn family_mismatch_remains_unknown_and_default_sends_nothing() {
    let (intent, key, _, _) = fixture(ConnectionTransport::NativeUdp);
    let mut manager = Manager::new(None, Duration::from_secs(10));
    manager
        .reconcile(snapshot(intent.clone()), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.discovery_snapshot().0, State::Disabled);
    assert!(manager.discovery_snapshot().1.is_empty());
    manager.withdraw(Reason::ShuttingDown).await;
    manager.enable_discovery(
        crate::EndpointDiscoveryOptions::new(vec!["[::1]:12345".parse().unwrap()]).unwrap(),
    );
    manager
        .reconcile(snapshot(intent), key, Instant::now())
        .await;
    time::timeout(Duration::from_secs(2), async {
        loop {
            let (state, sockets, _) = manager.discovery_snapshot();
            if !sockets.is_empty() {
                assert_eq!(state, State::Probing);
                assert!(sockets.iter().all(|s| s.observations.is_empty()));
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    manager.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn pending_stun_and_http_are_fenced_by_independent_configuration_deadline() {
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let observer = udp.local_addr().unwrap();
    let (intent, key, _, _) = fixture(ConnectionTransport::NativeUdp);
    let bound: std::net::SocketAddr = intent.listeners[0].bind.parse().unwrap();
    let mut manager = Manager::new(None, Duration::from_millis(400));
    manager.enable_discovery(crate::EndpointDiscoveryOptions::new(vec![observer]).unwrap());
    manager
        .reconcile(snapshot(intent), key, Instant::now())
        .await;
    let mut bytes = [0_u8; 2048];
    time::timeout(Duration::from_millis(200), udp.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let (_, _, guard) = manager.discovery_snapshot();
    let mut guard = guard.unwrap();
    let delayed_success = async {
        time::sleep(Duration::from_secs(2)).await;
        true
    };
    let accepted = tokio::select! {
        biased;
        _ = guard.changed() => false,
        accepted = delayed_success => accepted,
    };
    assert!(
        !accepted,
        "a pending HTTP response must not succeed after expiry"
    );
    // A fresh neighbor publication can also cancel this guard before lease
    // expiry. The independent watchdog must still eventually withdraw sockets.
    time::timeout(Duration::from_millis(700), async {
        while manager.discovery_snapshot().0 != State::Withdrawn {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(manager.discovery_snapshot().0, State::Withdrawn);
    assert!(manager.discovery_snapshot().1.is_empty());
    time::timeout(Duration::from_millis(500), async {
        loop {
            if tokio::net::UdpSocket::bind(bound).await.is_ok() {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    manager.withdraw(Reason::ShuttingDown).await;
}
