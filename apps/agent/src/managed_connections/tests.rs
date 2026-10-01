use super::*;
use meshway_dataplane::Identity;
use meshway_protocol::{
    ConnectionEndpoint, ConnectionListener, ConnectionPeer, IncomingConnectionApproval,
    OutgoingConnectionCandidate,
};

// These tests deliberately release and rebind listeners during reconcile. Keep
// their ports outside the usual auto-assigned client source-port range; otherwise
// concurrent real TCP dial fixtures can acquire the just-released ephemeral port.
fn listener_port(kind: ConnectionTransport) -> u16 {
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
        return port;
    }
}

pub(super) fn fixture(
    kind: ConnectionTransport,
) -> (NodeConnections, PrivateKey, NodeConnections, PrivateKey) {
    let a = Identity::generate().unwrap();
    let b = Identity::generate().unwrap();
    let a_id = "a".repeat(64);
    let b_id = "b".repeat(64);
    let port = listener_port(kind);
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

pub(super) fn snapshot(payload: NodeConnections) -> AgentConnectionsSnapshot {
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
    let targets = resolve_targets(intent)
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
            assert!(
                tokio::net::TcpListener::bind(&server.listeners[0].bind)
                    .await
                    .is_ok()
            );
        } else {
            assert!(
                tokio::net::UdpSocket::bind(&server.listeners[0].bind)
                    .await
                    .is_ok()
            );
        }
    }
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
    assert!(
        tokio::net::TcpListener::bind(&server.listeners[0].bind)
            .await
            .is_ok()
    );
    manager
        .reconcile(snapshot(revised.clone()), key.clone(), Instant::now())
        .await;
    assert_eq!(manager.observation().state, Phase::Applied);
    revised.valid = false;
    manager.invalidate_if_changed(&snapshot(revised.clone()));
    time::sleep(Duration::from_millis(50)).await;
    assert!(
        tokio::net::TcpListener::bind(&server.listeners[0].bind)
            .await
            .is_ok()
    );
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
    assert!(
        tokio::net::TcpListener::bind(&server.listeners[0].bind)
            .await
            .is_ok()
    );
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
    let rebound = tokio::net::TcpListener::bind(&server.listeners[0].bind).await;
    assert!(
        rebound.is_ok(),
        "failed rebind {}: {:?}",
        server.listeners[0].bind,
        rebound.as_ref().err()
    );
    drop(rebound);
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
    assert!(
        tokio::net::TcpListener::bind(&server.listeners[0].bind)
            .await
            .is_ok()
    );
}

#[cfg(unix)]
pub(super) fn tls_profiles() -> tempfile::TempDir {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let result = Command::new("openssl")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(result.status.success(), "openssl fixture generation failed");
    };
    run(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "ca-key.pem",
        "-out",
        "ca.pem",
        "-days",
        "1",
        "-subj",
        "/CN=Meshway test CA",
        "-addext",
        "basicConstraints=critical,CA:TRUE",
    ]);
    run(&[
        "req",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "key.pem",
        "-out",
        "server.csr",
        "-subj",
        "/CN=lax.mesh.test",
    ]);
    std::fs::write(dir.path().join("extensions"), "subjectAltName=DNS:lax.mesh.test\nbasicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n").unwrap();
    run(&[
        "x509",
        "-req",
        "-in",
        "server.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca-key.pem",
        "-CAcreateserial",
        "-out",
        "cert.pem",
        "-days",
        "1",
        "-extfile",
        "extensions",
    ]);
    for file in ["key.pem", "ca-key.pem"] {
        std::fs::set_permissions(
            dir.path().join(file),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
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
