use super::*;
use crate::managed_dataplane::{
    PacketPlan, TunOptions,
    tests::{MockDevice, fixtures, packet, snapshot},
    tun::Tun,
};
use meshway_dataplane::{
    LabConfig, OutgoingCandidate, PeerConfig, PrivateKey, Runtime, TransportKind,
};
use meshway_protocol::{AgentNodeResponse, NodeSummary};
use std::sync::Arc;
use tokio::sync::Notify;

#[tokio::test]
async fn heartbeat_new_revision_fences_real_packets_before_stalled_publication() {
    const LEASE: Duration = Duration::from_secs(30);
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (payloads, _) = fixtures(2);
    let mut payload = payloads[0].clone();
    let mut agent = Agent::prepare_enrollment(
        dir.path().join("agent.json"),
        &base,
        &"a".repeat(64),
        &payload.configuration.node.name,
    )
    .unwrap();
    agent.identity.node_id = Some(payload.node_id.clone());
    crate::state::write_identity(&agent.path, &agent.identity, true).unwrap();
    agent.load_connections().unwrap();
    payload.connections.public_key = Some(
        agent
            .connections
            .as_mut()
            .unwrap()
            .keys
            .prepare(&agent.path, &agent.identity)
            .unwrap()
            .into(),
    );
    let remote_identity = meshway_dataplane::Identity::generate().unwrap();
    for peer in payload
        .connections
        .peers
        .iter_mut()
        .chain(&mut payload.identities)
    {
        peer.public_key = remote_identity.public_key_hex();
    }
    let node = &payload.configuration.node;
    let now = chrono::Utc::now();
    let response = AgentNodeResponse {
        node: NodeSummary {
            id: node.id.clone(),
            name: node.name.clone(),
            country: node.country.clone(),
            mesh_ipv4: node.mesh_ipv4.clone(),
            mesh_ipv6: node.mesh_ipv6.clone(),
            configuration_revision: payload.revision + 1,
            acknowledged_revision: 0,
            agent_version: agent.identity.enrollment_version.clone(),
            registered_at: now,
            last_seen: now,
            revoked_at: None,
        },
    };
    let stalled = Arc::new(Notify::new());
    let signal = stalled.clone();
    let app = axum::Router::new()
        .route(
            "/api/v1/agents/heartbeat",
            axum::routing::post(move || {
                let response = AgentNodeResponse {
                    node: response.node.clone(),
                };
                async move { axum::Json(response) }
            }),
        )
        .route(
            "/api/v1/agent/connection-identity",
            axum::routing::post(move || {
                let signal = signal.clone();
                async move {
                    signal.notify_one();
                    std::future::pending::<
                        axum::Json<meshway_protocol::PublishConnectionIdentityResponse>,
                    >()
                    .await
                }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (device, _kernel, mut delivered) = MockDevice::new();
    let mut manager = Manager::new_dataplane(
        None,
        LEASE,
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        base,
        false,
    );
    manager.mock_tun(Tun::mock(
        device.clone(),
        &PacketPlan::compile(&payload, 1000).unwrap(),
    ));
    let key = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .application_key(&agent.path, &agent.identity)
        .unwrap();
    let fresh_at = Instant::now();
    manager
        .reconcile_dataplane(snapshot(payload.clone()), key, fresh_at)
        .await
        .unwrap();
    let remote = Runtime::start(LabConfig {
        node_id: payload.configuration.peers[0].id.clone(),
        private_key: PrivateKey::from_hex(&remote_identity.private_key_hex()).unwrap(),
        public_key: remote_identity.public_key_hex(),
        trusted_peers: vec![PeerConfig {
            node_id: payload.node_id.clone(),
            public_key: payload.connections.public_key.clone().unwrap(),
            transports: vec![TransportKind::NativeUdp],
        }],
        allowed_paths: payload
            .access_policy
            .grants
            .iter()
            .map(|g| g.path.clone())
            .collect(),
        listeners: vec![],
        outgoing: vec![OutgoingCandidate {
            peer: payload.node_id.clone(),
            transport: TransportKind::NativeUdp,
            endpoint: payload.connections.listeners[0].bind.clone(),
            priority: 0,
            id: Some("dial-1".into()),
        }],
        limits: Default::default(),
    })
    .await
    .unwrap();
    remote
        .wait_for_peer(&payload.node_id, Duration::from_secs(4))
        .await
        .unwrap();
    let reverse = payload.access_policy.grants[1].path.clone();
    let bytes = packet("100.64.0.2", "100.64.0.1", 1);
    device.allow_writes();
    remote
        .send_packet(reverse.clone(), bytes.clone())
        .await
        .unwrap();
    assert_eq!(
        time::timeout(Duration::from_secs(2), delivered.recv())
            .await
            .unwrap(),
        Some(bytes.clone())
    );
    device.inject_gate.acquire().await.unwrap().forget();
    time::timeout(Duration::from_secs(1), device.attempted.notified())
        .await
        .unwrap();
    remote
        .send_packet(reverse.clone(), bytes.clone())
        .await
        .unwrap();
    time::timeout(Duration::from_secs(2), device.attempted.notified())
        .await
        .unwrap();
    let mut lease = RuntimeLease::new().unwrap();
    let mut synced = false;
    let mut cache = None;
    {
        let tick = agent.managed_tick(&mut manager, &mut lease, &mut synced, &mut cache);
        tokio::pin!(tick);
        tokio::select! {result=&mut tick=>panic!("tick completed: {}",result.is_ok()),result=time::timeout(Duration::from_secs(2),stalled.notified())=>result.unwrap()};
        device.allow_writes();
        let _ = remote.send_packet(reverse, bytes).await;
        assert!(
            time::timeout(Duration::from_millis(150), delivered.recv())
                .await
                .is_err()
        );
    }
    assert!(fresh_at.elapsed() < LEASE / 2);
    assert_eq!(
        manager.observation().state,
        meshway_protocol::ConnectionRuntimeState::Withdrawn
    );
    assert!(!synced);
    manager.withdraw(Reason::ShuttingDown).await;
    remote.shutdown().await;
    server.abort();
}
