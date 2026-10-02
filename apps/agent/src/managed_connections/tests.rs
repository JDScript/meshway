use super::*;
use meshway_dataplane::Identity;
use meshway_protocol::{
    ConnectionEndpoint, ConnectionListener, ConnectionPeer, IncomingConnectionApproval,
    OutgoingConnectionCandidate,
};

#[tokio::test]
async fn interrupted_withdraw_retains_exact_generation_until_join() {
    let mut manager = Manager::new(None, Duration::from_secs(10));
    let (deadline, mut cancelled) = watch::channel(Some(Instant::now() + Duration::from_secs(10)));
    let (_, observed) = watch::channel(Observation::empty(Phase::Applied, Reason::None));
    let (_, discovery) = watch::channel(discovery::Publication::new(
        discovery::Source {
            revision: 7,
            digest: "a".repeat(64),
        },
        meshway_protocol::EndpointDiscoveryState::Disabled,
    ));
    let (stopping, stopped) = tokio::sync::oneshot::channel();
    let (release, barrier) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        cancelled.changed().await.unwrap();
        assert!(cancelled.borrow().is_none());
        stopping.send(()).unwrap();
        barrier.await.unwrap();
    });
    let task_id = task.id();
    manager.running = Some(Running {
        revision: 7,
        deadline,
        observed,
        task,
        discovery,
        discovery_task: None,
    });
    {
        let withdraw = manager.withdraw(Reason::ShuttingDown);
        tokio::pin!(withdraw);
        tokio::select! {
            _ = &mut withdraw => panic!("withdraw completed before session join"),
            _ = stopped => {},
        }
        // Drop the first teardown while its exact child is behind the barrier.
    }
    let running = manager
        .running
        .as_ref()
        .expect("in-progress owner retained");
    assert_eq!(running.revision, 7);
    assert_eq!(running.task.id(), task_id);
    assert!(!running.active());
    {
        let withdraw = manager.withdraw(Reason::ShuttingDown);
        tokio::pin!(withdraw);
        tokio::select! {
            _ = &mut withdraw => panic!("second withdraw skipped old join"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
        release.send(()).unwrap();
        withdraw.await;
    }
    assert!(manager.running.is_none());
}

// These tests deliberately release and rebind listeners during reconcile. Keep
// their ports outside the usual auto-assigned client source-port range; otherwise
// concurrent real TCP dial fixtures can acquire the just-released ephemeral port.
#[track_caller]
pub(crate) fn listener_port(kind: ConnectionTransport) -> u16 {
    static NEXT: std::sync::LazyLock<std::sync::atomic::AtomicU16> =
        std::sync::LazyLock::new(|| {
            let mut bytes = [0; 2];
            getrandom::fill(&mut bytes).unwrap();
            std::sync::atomic::AtomicU16::new(10_000 + u16::from_be_bytes(bytes) % 19_000)
        });
    loop {
        let port = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(port < 30_000, "test listener port budget exhausted");
        let Ok(_tcp) = std::net::TcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        if kind == ConnectionTransport::NativeUdp
            && std::net::UdpSocket::bind(("127.0.0.1", port)).is_err()
        {
            continue;
        }
        let previous = allocated_ports().lock().unwrap().insert(
            port,
            format!(
                "{} at {}",
                std::thread::current().name().unwrap_or("unnamed"),
                std::panic::Location::caller()
            ),
        );
        assert!(
            previous.is_none(),
            "fixture port allocated twice: {port}, prior={previous:?}"
        );
        return port;
    }
}

fn allocated_ports() -> &'static std::sync::Mutex<BTreeMap<u16, String>> {
    static PORTS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<u16, String>>> =
        std::sync::OnceLock::new();
    PORTS.get_or_init(Default::default)
}

/// Preserve the immediate post-shutdown assertion. Diagnostics are gathered only
/// after the first bind has failed; there is no wait/retry that could hide a leak.
async fn assert_udp_released(address: &str) {
    let rebound = tokio::net::UdpSocket::bind(address).await;
    if let Err(error) = rebound {
        bind_failure(address, "UDP", error).await;
    }
}

async fn assert_tcp_released(address: &str) {
    let rebound = tokio::net::TcpListener::bind(address).await;
    if let Err(error) = rebound {
        bind_failure(address, "TCP", error).await;
    }
}

async fn bind_failure(address: &str, protocol: &str, error: std::io::Error) -> ! {
    let port = address.parse::<SocketAddr>().unwrap().port();
    let allocation = allocated_ports().lock().unwrap().get(&port).cloned();
    let pid = std::process::id().to_string();
    eprintln!(
        "immediate {protocol} rebind failure: address={address} kind={:?} errno={:?} error={error}; allocation={allocation:?}; own PID={pid}",
        error.kind(),
        error.raw_os_error()
    );
    // Inspect only this test process, and only after the assertion failed.
    // `-a` intersects the PID and UDP-port selectors; no unrelated services.
    let mut command = tokio::process::Command::new("/usr/sbin/lsof");
    command
        .args([
            "-nP",
            "-w",
            "-b",
            "-a",
            "-p",
            &pid,
            &format!("-i{protocol}:{port}"),
        ])
        .kill_on_drop(true);
    match time::timeout(Duration::from_secs(3), command.output()).await {
        Ok(Ok(output)) => eprintln!(
            "owned {protocol} bindings: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        other => eprintln!("owned {protocol} binding diagnostics unavailable: {other:?}"),
    }
    panic!(
        "{protocol} listener {address} was not released by shutdown: {error} (errno {:?})",
        error.raw_os_error()
    );
}

pub(crate) fn fixture(
    kind: ConnectionTransport,
) -> (NodeConnections, PrivateKey, NodeConnections, PrivateKey) {
    fixture_at_port(kind, listener_port(kind))
}

pub(crate) fn fixture_at_port(
    kind: ConnectionTransport,
    port: u16,
) -> (NodeConnections, PrivateKey, NodeConnections, PrivateKey) {
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let a_id = "a".repeat(64);
    let b_id = "b".repeat(64);
    let base = |id: String, key: String| NodeConnections {
        schema_version: 1,
        revision: 1,
        node_id: id,
        public_key: Some(key),
        capabilities: crate::compiled_connection_capabilities(),
        valid: true,
        diagnostics: vec![],
        listeners: vec![],
        outgoing: vec![],
        incoming: vec![],
        peers: vec![],
    };
    let mut server = base(a_id.clone(), a.public_key_hex());
    let mut client = base(b_id.clone(), b.public_key_hex());
    server.peers.push(ConnectionPeer {
        node_id: b_id.clone(),
        public_key: b.public_key_hex(),
    });
    client.peers.push(ConnectionPeer {
        node_id: a_id.clone(),
        public_key: a.public_key_hex(),
    });
    server.listeners.push(ConnectionListener {
        id: "listener-a".into(),
        node_id: a_id.clone(),
        transport: kind,
        bind: format!("127.0.0.1:{port}"),
        advertised_endpoints: vec![ConnectionEndpoint {
            host: "localhost".into(),
            port,
        }],
        credential_ref: (kind == ConnectionTransport::Vless).then(|| "server".into()),
    });
    server.incoming.push(IncomingConnectionApproval {
        candidate_id: "reverse-sha-lax".into(),
        listener_id: "listener-a".into(),
        peer_node_id: b_id,
        transport: kind,
    });
    client.outgoing.push(OutgoingConnectionCandidate {
        id: "reverse-sha-lax".into(),
        listener_id: "listener-a".into(),
        peer_node_id: a_id,
        transport: kind,
        endpoint: ConnectionEndpoint {
            host: "localhost".into(),
            port,
        },
        priority: 0,
        credential_ref: (kind == ConnectionTransport::Vless).then(|| "client".into()),
    });
    (
        server,
        PrivateKey::from_hex(&a.private_key_hex()).unwrap(),
        client,
        PrivateKey::from_hex(&b.private_key_hex()).unwrap(),
    )
}

pub(crate) fn snapshot(payload: NodeConnections) -> AgentConnectionsSnapshot {
    AgentConnectionsSnapshot {
        digest: meshway_protocol::connection_digest(&payload).unwrap(),
        payload,
    }
}

pub(super) async fn session(
    intent: &NodeConnections,
    key: PrivateKey,
    path: Option<&std::path::Path>,
) -> session::Session {
    let targets = resolve_targets(intent, &sockets::SystemUnderlay)
        .await
        .unwrap_or_else(|_| panic!("resolve fixture"));
    session::Session::start(
        intent,
        key,
        profiles::load(path, intent).unwrap_or_else(|_| panic!("load fixture")),
        &targets,
    )
    .await
    .unwrap_or_else(|_| panic!("start fixture"))
}

#[tokio::test]
async fn native_udp_tcp_full_ids_reverse_dial_and_control_only() {
    for kind in [
        ConnectionTransport::NativeUdp,
        ConnectionTransport::NativeTcp,
    ] {
        let (server, skey, client, ckey) = fixture(kind);
        let mut a = session(&server, skey, None).await;
        let mut b = session(&client, ckey, None).await;
        a.runtime
            .wait_for_peer(&client.node_id, Duration::from_secs(4))
            .await
            .unwrap();
        b.runtime
            .wait_for_peer(&server.node_id, Duration::from_secs(4))
            .await
            .unwrap();
        a.runtime
            .probe_neighbor(&client.node_id, Duration::from_secs(2))
            .await
            .unwrap();
        b.runtime
            .probe_neighbor(&server.node_id, Duration::from_secs(2))
            .await
            .unwrap();
        let accepted = &a.runtime.peer_connections()[0];
        assert!(!accepted.initiator);
        assert_eq!(accepted.listener_id.as_deref(), Some("listener-a"));
        assert!(b.runtime.peer_connections()[0].initiator);
        assert!(
            a.runtime
                .probe(
                    vec![server.node_id.clone(), client.node_id.clone()],
                    b"forbidden data probe".to_vec()
                )
                .await
                .is_err()
        );
        assert!(
            b.runtime
                .send_packet(
                    vec![client.node_id.clone(), server.node_id.clone()],
                    vec![0; 20]
                )
                .await
                .is_err()
        );
        a.shutdown().await;
        b.shutdown().await;
        if kind == ConnectionTransport::NativeTcp {
            assert_tcp_released(&server.listeners[0].bind).await;
        } else {
            assert_udp_released(&server.listeners[0].bind).await;
        }
    }
}

#[tokio::test]
async fn closed_udp_socket_is_not_retained_by_parallel_tls_fixture_creation() {
    // No Meshway Runtime participates: isolate OS descriptor lifetime from core
    // shutdown while exercising the suite's real certificate helper in parallel.
    let certificates = tokio::task::spawn_blocking(tls_profiles);
    let address = format!(
        "127.0.0.1:{}",
        listener_port(ConnectionTransport::NativeUdp)
    );
    for _ in 0..5000 {
        // Each successful assertion's socket is dropped before the next bind.
        // Fail on the first unsuccessful bind; never retry a failed assertion.
        assert_udp_released(&address).await;
        tokio::task::yield_now().await;
    }
    let first = certificates.await.unwrap();
    let second = tls_profiles();
    assert_ne!(
        std::fs::read(first.path().join("ca.pem")).unwrap(),
        std::fs::read(second.path().join("ca.pem")).unwrap(),
        "each fixture still needs a fresh CA for wrong-root authentication tests"
    );
}

#[tokio::test]
async fn converged_dns_ip_candidates_apply_with_preferred_metadata_and_keep_other_listener() {
    for kind in [
        ConnectionTransport::NativeUdp,
        ConnectionTransport::NativeTcp,
    ] {
        let (server, server_key, mut client, client_key) = fixture(kind);
        let preferred = client.outgoing[0].clone();
        let physical: SocketAddr = server.listeners[0].bind.parse().unwrap();
        let mut alias = preferred.clone();
        alias.id = "ip-standby".into();
        alias.endpoint.host = physical.ip().to_string();
        alias.priority = preferred.priority + 1;
        client.outgoing.push(alias.clone());
        let own_address = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, listener_port(kind)));
        client.listeners.push(ConnectionListener {
            id: "unrelated-local-listener".into(),
            node_id: client.node_id.clone(),
            transport: kind,
            bind: own_address.to_string(),
            advertised_endpoints: vec![ConnectionEndpoint {
                host: "localhost".into(),
                port: own_address.port(),
            }],
            credential_ref: None,
        });
        // Controlled resolver snapshot for the single-A-record alias case.
        // The real reconcile below also exercises the host's localhost resolver,
        // which can additionally contain a distinct IPv6 address.
        let resolved = BTreeMap::from([
            (preferred.id.clone(), vec![physical]),
            (alias.id.clone(), vec![physical]),
        ]);
        let config = core_config(&client, client_key.clone(), &resolved)
            .unwrap_or_else(|_| panic!("converged approved targets must form a valid core plan"));
        assert_eq!(config.outgoing.len(), 1);
        assert_eq!(
            config.outgoing[0].id.as_deref(),
            Some(preferred.id.as_str())
        );
        assert_eq!(config.outgoing[0].priority, preferred.priority);
        assert_eq!(config.listeners.len(), 1);
        assert!(config.allowed_paths.is_empty());

        let mut remote = session(&server, server_key, None).await;
        let mut manager = Manager::new(None, Duration::from_secs(30));
        manager
            .reconcile(snapshot(client.clone()), client_key.clone(), Instant::now())
            .await;
        assert_eq!(manager.observation().state, Phase::Applied);
        let generation = manager.running.as_ref().unwrap().task.id();
        time::timeout(Duration::from_secs(8), async {
            loop {
                if manager
                    .observation()
                    .peers
                    .iter()
                    .any(|peer| peer.probe_fresh)
                {
                    break;
                }
                time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("preferred native candidate must complete a real neighbor probe");
        let observed = manager.observation();
        let authenticated = observed.peers.iter().find(|peer| peer.probe_fresh).unwrap();
        assert_eq!(
            authenticated.candidate_id.as_deref(),
            Some(preferred.id.as_str())
        );
        let standby = observed
            .peers
            .iter()
            .find(|peer| peer.candidate_id.as_deref() == Some(alias.id.as_str()))
            .unwrap();
        assert_eq!(
            standby.state,
            meshway_protocol::ConnectionNeighborState::Waiting
        );
        assert_eq!(standby.reason, Reason::None);
        assert!(!standby.probe_fresh);
        assert_eq!(
            observed.listeners[0].listener_id,
            "unrelated-local-listener"
        );
        client.revision += 1;
        manager
            .reconcile(snapshot(client), client_key, Instant::now())
            .await;
        assert_eq!(manager.observation().state, Phase::Applied);
        assert_eq!(manager.running.as_ref().unwrap().task.id(), generation);
        if kind == ConnectionTransport::NativeTcp {
            assert!(tokio::net::TcpListener::bind(own_address).await.is_err());
        } else {
            assert!(tokio::net::UdpSocket::bind(own_address).await.is_err());
        }
        manager.withdraw(Reason::ShuttingDown).await;
        remote.shutdown().await;
    }
}

#[test]
fn native_target_dedup_precedes_count_limits_and_preserves_peer_carrier_boundaries() {
    for kind in [
        ConnectionTransport::NativeUdp,
        ConnectionTransport::NativeTcp,
    ] {
        let (_, _, mut client, key) = fixture(kind);
        let template = client.outgoing[0].clone();
        let physical = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, template.endpoint.port));
        client.outgoing = (0..40)
            .map(|index| {
                let mut candidate = template.clone();
                candidate.id = format!("alias-{index:02}");
                candidate.priority = index;
                candidate
            })
            .collect();
        let mut other_carrier = template.clone();
        other_carrier.id = "other-carrier".into();
        other_carrier.priority = 40;
        other_carrier.transport = if kind == ConnectionTransport::NativeUdp {
            ConnectionTransport::NativeTcp
        } else {
            ConnectionTransport::NativeUdp
        };
        client.outgoing.push(other_carrier);
        let identity = Identity::generate().unwrap();
        let other_peer = "c".repeat(64);
        client.peers.push(ConnectionPeer {
            node_id: other_peer.clone(),
            public_key: identity.public_key_hex(),
        });
        let mut distinct = template;
        distinct.id = "other-peer".into();
        distinct.priority = 41;
        distinct.peer_node_id = other_peer;
        client.outgoing.push(distinct);
        let resolved = client
            .outgoing
            .iter()
            .map(|candidate| (candidate.id.clone(), vec![physical]))
            .collect();
        let config = core_config(&client, key, &resolved)
            .unwrap_or_else(|_| panic!("dedup must precede the native count limit"));
        assert_eq!(config.outgoing.len(), 3);
        assert_eq!(config.outgoing[0].id.as_deref(), Some("alias-00"));
        assert_eq!(config.outgoing[1].id.as_deref(), Some("other-carrier"));
        assert_eq!(config.outgoing[2].id.as_deref(), Some("other-peer"));
    }
}

#[tokio::test]
async fn listener_scopes_and_pinned_peer_keys_fail_closed() {
    for kind in [
        ConnectionTransport::NativeUdp,
        ConnectionTransport::NativeTcp,
    ] {
        let (mut server, skey, mut client, ckey) = fixture(kind);
        let mut second = server.listeners[0].clone();
        second.id = "listener-b".into();
        second.bind = "127.0.0.1:0".into();
        server.listeners.push(second);
        let mut a = session(&server, skey, None).await;
        let port = a
            .runtime
            .bound_listeners()
            .iter()
            .find(|listener| listener.id.as_deref() == Some("listener-b"))
            .unwrap()
            .address
            .port();
        client.outgoing[0].endpoint = ConnectionEndpoint {
            host: "127.0.0.1".into(),
            port,
        };
        let mut b = session(&client, ckey.clone(), None).await;
        assert!(
            b.runtime
                .wait_for_peer(&server.node_id, Duration::from_millis(700))
                .await
                .is_err()
        );
        assert!(a.runtime.connected_peers().is_empty());
        b.shutdown().await;
        client.outgoing[0].endpoint.port = server.listeners[0]
            .bind
            .parse::<SocketAddr>()
            .unwrap()
            .port();
        client.peers[0].public_key = Identity::generate().unwrap().public_key_hex();
        let mut b = session(&client, ckey, None).await;
        assert!(
            b.runtime
                .wait_for_peer(&server.node_id, Duration::from_millis(700))
                .await
                .is_err()
        );
        assert!(a.runtime.connected_peers().is_empty());
        a.shutdown().await;
        b.shutdown().await;
    }
}

#[tokio::test]
async fn reconcile_revision_only_preserves_generation_invalid_and_lease_close_ports() {
    let (server, key, _, _) = fixture(ConnectionTransport::NativeTcp);
    let mut manager = Manager::new(None, Duration::from_millis(600));
    manager
        .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Applied);
    let task = manager.running.as_ref().unwrap().task.id();
    let mut revised = server.clone();
    revised.revision += 1;
    manager
        .reconcile(snapshot(revised.clone()), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.running.as_ref().unwrap().task.id(), task);
    let (_, report_guard) = manager.report_snapshot();
    let report_guard = report_guard.unwrap();
    assert!(report_guard.current());
    time::sleep(Duration::from_millis(800)).await;
    assert!(
        !report_guard.current(),
        "an expired generation must fence pending reports"
    );
    assert_eq!(manager.observation().state, Phase::Withdrawn);
    assert_eq!(manager.observation().reason, Reason::LeaseExpired);
    assert_tcp_released(&server.listeners[0].bind).await;
    manager
        .reconcile(snapshot(revised.clone()), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Applied);
    revised.valid = false;
    manager.invalidate_if_changed(&snapshot(revised.clone()));
    time::sleep(Duration::from_millis(50)).await;
    assert_tcp_released(&server.listeners[0].bind).await;
    manager
        .reconcile(snapshot(revised), key, Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Error);
    assert_eq!(manager.observation().reason, Reason::InvalidConfiguration);
}

#[tokio::test]
async fn missing_profile_withdraws_existing_plan_and_empty_plan_applies() {
    let (server, key, _, _) = fixture(ConnectionTransport::NativeTcp);
    let mut manager = Manager::new(None, Duration::from_secs(10));
    manager
        .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
        .await;
    let mut changed = server.clone();
    changed.listeners[0].transport = ConnectionTransport::Vless;
    changed.listeners[0].credential_ref = Some("missing".into());
    changed.incoming[0].transport = ConnectionTransport::Vless;
    manager
        .reconcile(snapshot(changed), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Error);
    assert_eq!(manager.observation().reason, Reason::ProfileUnavailable);
    assert!(manager.observation().listeners.is_empty());
    assert_tcp_released(&server.listeners[0].bind).await;
    let mut empty = server;
    empty.listeners.clear();
    empty.incoming.clear();
    empty.peers.clear();
    manager
        .reconcile(snapshot(empty), key, Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Applied);
    manager.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn failed_dns_or_atomic_bind_withdraws_old_listeners() {
    let (server, key, client, _) = fixture(ConnectionTransport::NativeTcp);
    let mut manager = Manager::new(None, Duration::from_secs(10));
    manager
        .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
        .await;
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut changed = server.clone();
    let mut listener = server.listeners[0].clone();
    listener.id = "occupied".into();
    listener.bind = occupied.local_addr().unwrap().to_string();
    changed.listeners.push(listener);
    manager
        .reconcile(snapshot(changed), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Error);
    assert_eq!(manager.observation().reason, Reason::BindFailed);
    assert_tcp_released(&server.listeners[0].bind).await;
    manager
        .reconcile(snapshot(server.clone()), key.clone(), Instant::now())
        .await;
    let mut changed = server.clone();
    changed.outgoing.push(OutgoingConnectionCandidate {
        id: "unresolved".into(),
        listener_id: "remote".into(),
        peer_node_id: client.node_id,
        transport: ConnectionTransport::NativeTcp,
        endpoint: ConnectionEndpoint {
            host: "meshway-test-does-not-exist.invalid".into(),
            port: 8421,
        },
        priority: 0,
        credential_ref: None,
    });
    manager
        .reconcile(snapshot(changed), key, Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Error);
    assert_eq!(manager.observation().reason, Reason::ResolveFailed);
    assert!(manager.observation().listeners.is_empty());
    assert_tcp_released(&server.listeners[0].bind).await;
}

#[cfg(unix)]
pub(super) fn tls_profiles() -> tempfile::TempDir {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
        KeyPair, KeyUsagePurpose,
    };
    let dir = tempfile::tempdir().unwrap();
    // Do not fork/posix_spawn from the parallel socket test process. On macOS,
    // spawn temporarily retains references to even CLOEXEC sockets until exec
    // closes them, making immediate post-close bind fail with EADDRINUSE. This
    // in-process fixture preserves unique CAs and keys without serializing tests.
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Meshway test CA");
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    meshway_protocol::local::atomic_write(
        &dir.path().join("ca-key.pem"),
        ca_key.serialize_pem().as_bytes(),
        false,
    )
    .unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut leaf_params = CertificateParams::new(vec!["lax.mesh.test".into()]).unwrap();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "lax.mesh.test");
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&key, &issuer).unwrap();
    std::fs::write(dir.path().join("ca.pem"), ca.pem()).unwrap();
    std::fs::write(dir.path().join("cert.pem"), leaf.pem()).unwrap();
    meshway_protocol::local::atomic_write(
        &dir.path().join("key.pem"),
        key.serialize_pem().as_bytes(),
        false,
    )
    .unwrap();
    let value = serde_json::json!({"version":1,"profiles":[
        {"id":"server","role":"server","uuid":"11111111-1111-4111-8111-111111111111","certificate":"cert.pem","private_key":"key.pem"},
        {"id":"client","role":"client","uuid":"11111111-1111-4111-8111-111111111111","ca_certificate":"ca.pem","server_name":"lax.mesh.test"}
    ]});
    meshway_protocol::local::atomic_write(
        &dir.path().join("profiles.json"),
        &serde_json::to_vec(&value).unwrap(),
        false,
    )
    .unwrap();
    dir
}

#[cfg(unix)]
#[tokio::test]
async fn tls_vless_neighbors_verify_san_ca_and_profile_reload() {
    let dir = tls_profiles();
    let path = dir.path().join("profiles.json");
    let (server, skey, client, ckey) = fixture(ConnectionTransport::Vless);
    let mut a = session(&server, skey.clone(), Some(&path)).await;
    let mut b = session(&client, ckey.clone(), Some(&path)).await;
    a.runtime
        .wait_for_peer(&client.node_id, Duration::from_secs(5))
        .await
        .unwrap();
    b.runtime
        .wait_for_peer(&server.node_id, Duration::from_secs(5))
        .await
        .unwrap();
    a.runtime
        .probe_neighbor(&client.node_id, Duration::from_secs(2))
        .await
        .unwrap();
    b.runtime
        .probe_neighbor(&server.node_id, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(
        a.runtime.peer_connections()[0].listener_id.as_deref(),
        Some("listener-a")
    );
    b.shutdown().await;
    a.shutdown().await;
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["profiles"][1]["server_name"] = "wrong.mesh.test".into();
    meshway_protocol::local::atomic_write(&path, &serde_json::to_vec(&value).unwrap(), true)
        .unwrap();
    let mut a = session(&server, skey.clone(), Some(&path)).await;
    let mut b = session(&client, ckey.clone(), Some(&path)).await;
    assert!(
        b.runtime
            .wait_for_peer(&server.node_id, Duration::from_millis(900))
            .await
            .is_err()
    );
    assert!(a.runtime.connected_peers().is_empty());
    let failed = b.test_probe_observations(&client).await;
    assert_eq!(
        failed[0].state,
        meshway_protocol::ConnectionNeighborState::Unreachable
    );
    assert_eq!(failed[0].reason, Reason::AuthenticationFailed);
    assert!(!failed[0].probe_fresh);
    b.shutdown().await;
    a.shutdown().await;
    let other = tls_profiles();
    std::fs::copy(other.path().join("ca.pem"), dir.path().join("wrong-ca.pem")).unwrap();
    value["profiles"][1]["server_name"] = "lax.mesh.test".into();
    value["profiles"][1]["ca_certificate"] = "wrong-ca.pem".into();
    meshway_protocol::local::atomic_write(&path, &serde_json::to_vec(&value).unwrap(), true)
        .unwrap();
    let mut a = session(&server, skey.clone(), Some(&path)).await;
    let mut b = session(&client, ckey, Some(&path)).await;
    assert!(
        b.runtime
            .wait_for_peer(&server.node_id, Duration::from_millis(900))
            .await
            .is_err()
    );
    b.shutdown().await;
    a.shutdown().await;
    let mut manager = Manager::new(Some(path.clone()), Duration::from_secs(10));
    manager
        .reconcile(snapshot(server.clone()), skey.clone(), Instant::now())
        .await;
    let task = manager.running.as_ref().unwrap().task.id();
    value["profiles"][0]["uuid"] = "22222222-2222-4222-8222-222222222222".into();
    meshway_protocol::local::atomic_write(&path, &serde_json::to_vec(&value).unwrap(), true)
        .unwrap();
    manager
        .reconcile(snapshot(server), skey, Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Applied);
    assert_ne!(manager.running.as_ref().unwrap().task.id(), task);
    manager.withdraw(Reason::ShuttingDown).await;
}

#[cfg(unix)]
#[test]
fn profiles_reject_unknown_wrong_role_permissions_and_bad_material() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tls_profiles();
    let path = dir.path().join("profiles.json");
    let (mut server, _, _, _) = fixture(ConnectionTransport::Vless);
    assert!(profiles::load(Some(&path), &server).is_ok());
    let alias = dir.path().join("profile-alias.json");
    symlink(&path, &alias).unwrap();
    assert!(profiles::load(Some(&alias), &server).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(profiles::load(Some(&path), &server).is_err());
    std::fs::remove_file(&alias).unwrap();
    server.listeners[0].credential_ref = Some("client".into());
    assert!(profiles::load(Some(&path), &server).is_err());
    server.listeners[0].credential_ref = Some("server".into());
    std::fs::set_permissions(
        dir.path().join("key.pem"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(profiles::load(Some(&path), &server).is_err());
    std::fs::set_permissions(
        dir.path().join("key.pem"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let key = std::fs::read(dir.path().join("key.pem")).unwrap();
    std::fs::write(dir.path().join("key.pem"), "invalid PEM material").unwrap();
    assert!(profiles::load(Some(&path), &server).is_err());
    std::fs::write(dir.path().join("key.pem"), key).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["profiles"][0]["allow_insecure"] = true.into();
    meshway_protocol::local::atomic_write(&path, &serde_json::to_vec(&value).unwrap(), true)
        .unwrap();
    assert!(profiles::load(Some(&path), &server).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(profiles::load(Some(&path), &server).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn tls_vless_listener_admission_is_scoped_before_noise_confirmation() {
    let dir = tls_profiles();
    let path = dir.path().join("profiles.json");
    let (mut server, skey, mut client, ckey) = fixture(ConnectionTransport::Vless);
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let mut unapproved = server.listeners[0].clone();
    unapproved.id = "listener-b".into();
    unapproved.bind = address.to_string();
    server.listeners.push(unapproved);
    client.outgoing[0].endpoint = ConnectionEndpoint {
        host: "localhost".into(),
        port: address.port(),
    };
    let mut a = session(&server, skey, Some(&path)).await;
    let mut b = session(&client, ckey, Some(&path)).await;
    assert!(
        b.runtime
            .wait_for_peer(&server.node_id, Duration::from_millis(900))
            .await
            .is_err()
    );
    assert!(a.runtime.connected_peers().is_empty());
    let failed = b.test_probe_observations(&client).await;
    assert_eq!(failed[0].reason, Reason::AuthenticationFailed);
    assert!(!failed[0].probe_fresh);
    b.shutdown().await;
    a.shutdown().await;
}
/// Injection boundary for report races; the recovery regression separately uses
/// real Manager sessions, normal probes and a real Controller.
pub(crate) fn report_generation(
    value: Observation,
    lease: Duration,
) -> (
    Manager,
    watch::Sender<Observation>,
    watch::Sender<Option<Instant>>,
) {
    let (observations, observed) = watch::channel(value);
    let (renew, mut deadlines) = watch::channel(Some(Instant::now() + lease));
    let (_, discovery) = watch::channel(discovery::Publication::new(
        discovery::Source {
            revision: 1,
            digest: "fixture".into(),
        },
        meshway_protocol::EndpointDiscoveryState::Disabled,
    ));
    let task = tokio::spawn(async move {
        loop {
            let deadline = *deadlines.borrow_and_update();
            let Some(deadline) = deadline else {
                return;
            };
            tokio::select! {
                _ = time::sleep_until(deadline) => return,
                changed = deadlines.changed() => if changed.is_err() { return; },
            }
        }
    });
    let mut manager = Manager::new(None, lease);
    manager.running = Some(Running {
        revision: 1,
        deadline: renew.clone(),
        observed,
        task,
        discovery,
        discovery_task: None,
    });
    (manager, observations, renew)
}

#[tokio::test]
async fn runtime_report_guard_negative_withdraw_and_generation_fences() {
    let mut value = Observation::empty(Phase::Applied, Reason::None);
    value
        .peers
        .push(meshway_protocol::ConnectionNeighborObservation {
            peer_node_id: "b".repeat(64),
            transport: ConnectionTransport::NativeTcp,
            initiator_node_id: "a".repeat(64),
            listener_id: Some("listener".into()),
            candidate_id: Some("candidate".into()),
            state: meshway_protocol::ConnectionNeighborState::Authenticated,
            reason: Reason::None,
            probe_fresh: true,
            probe_rtt_ms: Some(200),
        });
    let (mut manager, observations, deadlines) =
        report_generation(value.clone(), Duration::from_secs(30));
    let (_, proof) = manager.report_snapshot();
    let mut proof = proof.unwrap();
    observations.send_modify(|value| {
        value.peers[0].state = meshway_protocol::ConnectionNeighborState::Unreachable;
        value.peers[0].reason = Reason::ProbeFailed;
        value.peers[0].probe_fresh = false;
        value.peers[0].probe_rtt_ms = None;
    });
    assert!(!proof.current());
    time::timeout(Duration::from_millis(100), proof.changed())
        .await
        .unwrap();
    observations.send_replace(value.clone());
    assert!(!proof.current());
    let (_, proof) = manager.report_snapshot();
    let mut proof = proof.unwrap();
    observations.send_replace(Observation::empty(
        Phase::Withdrawn,
        Reason::TransportFailed,
    ));
    // Even if a later publication recovers before the reader runs, the old watch
    // version must not become valid again.
    observations.send_replace(value.clone());
    assert!(!proof.current());
    time::timeout(Duration::from_millis(100), proof.changed())
        .await
        .unwrap();
    let (_, proof) = manager.report_snapshot();
    let proof = proof.unwrap();
    deadlines.send_replace(None);
    deadlines.send_replace(Some(Instant::now() + Duration::from_secs(30)));
    assert!(!proof.current());
    manager.withdraw(Reason::ShuttingDown).await;
    assert!(!proof.current());
}
#[tokio::test(start_paused = true)]
async fn runtime_report_pending_wait_preserves_witness_for_renewal_aba_same_value_and_closed() {
    for mode in 0..5 {
        let original = Some(Instant::now() + Duration::from_secs(30));
        let (deadline_sender, deadline) = watch::channel(original);
        let (observation_sender, observed) =
            watch::channel(Observation::empty(Phase::Applied, Reason::None));
        let mut guard = ReportGuard {
            deadline,
            observed,
            observation_expiry: None,
        };
        {
            let changed = guard.changed();
            tokio::pin!(changed);
            std::future::poll_fn(|cx| {
                use std::future::Future;
                assert!(changed.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            match mode {
                0 => {
                    deadline_sender.send_replace(Some(Instant::now() + Duration::from_secs(60)));
                }
                1 => {
                    deadline_sender.send_replace(None);
                    deadline_sender.send_replace(original);
                }
                2 => {
                    deadline_sender.send_replace(original);
                }
                3 => drop(deadline_sender),
                _ => drop(observation_sender),
            }
            changed.await;
        }
        assert!(
            !guard.current() && !guard.generation_current(),
            "pending notification mode {mode} must permanently invalidate the original token"
        );
        // A second wait must not consume the evidence or restore the token.
        guard.changed().await;
        assert!(!guard.current() && !guard.generation_current());
    }
}
