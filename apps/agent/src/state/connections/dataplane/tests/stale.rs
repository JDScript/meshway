//! A known stale revision must stop real packet delivery before a stalled retry
//! GET, independently of the original generation's still-unexpired lease.

use super::*;
use axum::{Json, extract::State as HttpState, http::StatusCode, response::IntoResponse};
use meshway_dataplane::{
    LabConfig, OutgoingCandidate, PeerConfig, PrivateKey, Runtime, TransportKind,
};
use meshway_protocol::PublishConnectionIdentityRequest;
use tokio::{sync::Notify, time};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Evidence {
    ConfigurationAck,
    ConnectionsAck,
    Publication,
}

struct View {
    snapshot: MeshExecutionSnapshot,
    armed: Option<Evidence>,
    stall_get: bool,
    armed_gets: usize,
    configuration_acks: usize,
    connections_acks: usize,
}

fn withdraw_grants(snapshot: &mut MeshExecutionSnapshot) {
    snapshot.payload.revision += 1;
    snapshot.payload.connections.revision += 1;
    snapshot.payload.paths.clear();
    snapshot.payload.identities = snapshot.payload.connections.peers.clone();
    snapshot.digest = mesh_execution_digest(&snapshot.payload).unwrap();
}

impl View {
    fn stale_at(&mut self, evidence: Evidence) -> bool {
        if self.armed != Some(evidence) {
            return false;
        }
        assert!(!self.stall_get, "stale evidence is emitted exactly once");
        withdraw_grants(&mut self.snapshot);
        self.stall_get = true;
        true
    }
}

struct Controller {
    view: Mutex<View>,
    stalled_get: Notify,
}

async fn publish(
    HttpState(controller): HttpState<Arc<Controller>>,
    Json(request): Json<PublishConnectionIdentityRequest>,
) -> Json<PublishConnectionIdentityResponse> {
    let mut view = controller.view.lock().unwrap();
    assert_eq!(
        Some(&request.public_key),
        view.snapshot.payload.connections.public_key.as_ref()
    );
    assert_eq!(
        request.capabilities,
        view.snapshot.payload.connections.capabilities
    );
    view.stale_at(Evidence::Publication);
    Json(PublishConnectionIdentityResponse {
        revision: view.snapshot.payload.revision,
        node_id: view.snapshot.payload.node_id.clone(),
        public_key: request.public_key,
        capabilities: request.capabilities,
    })
}

async fn dataplane(
    HttpState(controller): HttpState<Arc<Controller>>,
) -> Json<MeshExecutionResponse> {
    let snapshot = {
        let mut view = controller.view.lock().unwrap();
        if view.armed.is_some() {
            view.armed_gets += 1;
        }
        (!view.stall_get).then(|| view.snapshot.clone())
    };
    if let Some(snapshot) = snapshot {
        return Json(managed(snapshot));
    }
    controller.stalled_get.notify_one();
    std::future::pending().await
}

async fn configuration_ack(
    HttpState(controller): HttpState<Arc<Controller>>,
    Json(request): Json<AcknowledgeConfigurationRequest>,
) -> axum::response::Response {
    let mut view = controller.view.lock().unwrap();
    assert_eq!(request.revision, view.snapshot.payload.revision);
    assert_eq!(request.digest, projections(&view.snapshot).0.digest);
    view.configuration_acks += 1;
    if view.stale_at(Evidence::ConfigurationAck) {
        return StatusCode::CONFLICT.into_response();
    }
    Json(AcknowledgeConfigurationResponse {
        revision: request.revision,
        digest: request.digest,
    })
    .into_response()
}

async fn connections_ack(
    HttpState(controller): HttpState<Arc<Controller>>,
    Json(request): Json<AcknowledgeAgentConnectionsRequest>,
) -> axum::response::Response {
    let mut view = controller.view.lock().unwrap();
    assert_eq!(request.revision, view.snapshot.payload.revision);
    assert_eq!(
        request.digest,
        connection_digest(&view.snapshot.payload.connections).unwrap()
    );
    view.connections_acks += 1;
    if view.stale_at(Evidence::ConnectionsAck) {
        return StatusCode::CONFLICT.into_response();
    }
    Json(AcknowledgeAgentConnectionsResponse {
        revision: request.revision,
        digest: request.digest,
        status: ConnectionReceiptStatus::Saved,
    })
    .into_response()
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Complete one real E2E delivery, then leave a second plaintext write blocked
/// inside the mock TUN. Both stale-control and terminal-auth tests use this flow.
pub(super) async fn blocked_peer(
    payload: &NodeMeshExecution,
    remote_identity: meshway_dataplane::Identity,
    device: &MockDevice,
    delivered: &mut tokio::sync::mpsc::Receiver<Vec<u8>>,
) -> (Runtime, Vec<u8>) {
    let remote = Runtime::start(LabConfig {
        node_id: payload.peers[0].id.clone(),
        private_key: PrivateKey::from_hex(&remote_identity.private_key_hex()).unwrap(),
        public_key: remote_identity.public_key_hex(),
        trusted_peers: vec![PeerConfig {
            node_id: payload.node_id.clone(),
            public_key: payload.connections.public_key.clone().unwrap(),
            transports: vec![TransportKind::NativeUdp],
        }],
        allowed_paths: payload
            .paths
            .iter()
            .map(|grant| grant.nodes.clone())
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
    let reverse = payload.paths[1].nodes.clone();
    let bytes = crate::managed_dataplane::tests::packet("100.64.0.2", "100.64.0.1", 1);
    let mut remote_payload = payload.clone();
    let node = remote_payload.peers.remove(0);
    remote_payload.peers.push(remote_payload.node.clone());
    remote_payload.node = node;
    remote_payload.node_id = remote_payload.node.id.clone();
    let plan = crate::managed_dataplane::PacketPlan::compile(&remote_payload, 1000).unwrap();
    use meshway_dataplane::packet_io::Admission;
    let envelope = plan.encapsulate(&reverse, &bytes).unwrap();
    device.allow_writes();
    remote
        .send_packet(reverse.clone(), envelope.clone())
        .await
        .unwrap();
    assert_eq!(
        time::timeout(Duration::from_secs(2), delivered.recv())
            .await
            .unwrap(),
        Some(bytes.clone())
    );
    device.inject_gate.acquire().await.unwrap().forget();
    // Consume the warm-up notification before arranging a distinct blocked write.
    time::timeout(Duration::from_secs(1), device.attempted.notified())
        .await
        .unwrap();
    remote.send_packet(reverse, envelope.clone()).await.unwrap();
    time::timeout(Duration::from_secs(2), device.attempted.notified())
        .await
        .unwrap();
    (remote, envelope)
}

async fn known_stale_evidence_cancels_delivery(evidence: Evidence, saved_ahead: bool) {
    const LEASE: Duration = Duration::from_secs(30);
    let dir = directory();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (mut agent, mut payload) = enrolled(dir.path(), &base);
    let remote_identity = meshway_dataplane::Identity::generate().unwrap();
    let remote_id = payload.peers[0].id.clone();
    for peer in payload
        .connections
        .peers
        .iter_mut()
        .chain(&mut payload.identities)
    {
        assert_eq!(peer.node_id, remote_id);
        peer.public_key = remote_identity.public_key_hex();
    }
    let original = snapshot(payload.clone());
    let controller = Arc::new(Controller {
        view: Mutex::new(View {
            snapshot: original.clone(),
            armed: None,
            stall_get: false,
            armed_gets: 0,
            configuration_acks: 0,
            connections_acks: 0,
        }),
        stalled_get: Notify::new(),
    });
    let app = axum::Router::new()
        .route(
            "/api/v1/agent/connection-identity",
            axum::routing::post(publish),
        )
        .route(
            "/api/v1/agents/mesh-execution",
            axum::routing::post(dataplane),
        )
        .route(
            "/api/v1/agents/configuration",
            axum::routing::get(
                |HttpState(controller): HttpState<Arc<Controller>>| async move {
                    Json(projections(&controller.view.lock().unwrap().snapshot).0)
                },
            ),
        )
        .route(
            "/api/v1/agents/mesh-execution/ack",
            axum::routing::post(execution_ack),
        )
        .route(
            "/api/v1/agents/configuration/ack",
            axum::routing::post(configuration_ack),
        )
        .route(
            "/api/v1/agent/connections/ack",
            axum::routing::post(connections_ack),
        )
        .with_state(controller.clone());
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap()
    }));
    let mut cache = State::load(&mut agent).unwrap();
    let (device, _from_kernel, mut delivered) = MockDevice::new();
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
    let synced = cache
        .sync(&mut agent, &mut manager, &RUNTIME)
        .await
        .unwrap();
    let fresh_at = synced.fresh_at;
    let key = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .application_key(&agent.path, &agent.identity)
        .unwrap();
    manager
        .reconcile_dataplane(synced.snapshot, key, fresh_at)
        .await
        .unwrap();

    // The peer actively dials this Agent's approved listener, then sends the
    // explicitly granted reverse business path through the real Noise/UDP core.
    let (remote, envelope) = blocked_peer(&payload, remote_identity, &device, &mut delivered).await;
    let reverse = payload.paths[1].nodes.clone();
    assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);

    if saved_ahead {
        // A saved cache is not the installed generation. Deliberately advance
        // the durable floor while preserving active R to catch cache-based tests.
        let mut newer = original.clone();
        withdraw_grants(&mut newer);
        let (configuration, connections) = projections(&newer);
        cache.persist(&agent, newer).unwrap();
        agent.persist_configuration(configuration).unwrap();
        agent
            .connections
            .as_mut()
            .unwrap()
            .persist(&agent.path, &agent.identity, connections)
            .unwrap();
        assert_eq!(
            cache.cache.as_ref().unwrap().snapshot.payload.revision,
            payload.revision + 1
        );
        assert_eq!(
            manager.snapshot.as_ref().unwrap().payload.revision,
            payload.revision
        );
    }
    controller.view.lock().unwrap().armed = Some(evidence);
    let started = Instant::now();
    {
        let sync = cache.sync(&mut agent, &mut manager, &RUNTIME);
        tokio::pin!(sync);
        tokio::select! {
            result = &mut sync => panic!("retry GET unexpectedly completed: {}", result.is_ok()),
            result = time::timeout(Duration::from_secs(2), controller.stalled_get.notified()) => result.unwrap(),
        }
        // The GET is deliberately still pending. Opening the mock write gate
        // must not release the plaintext already queued in the old generation.
        device.allow_writes();
        let _ = remote.send_packet(reverse, envelope).await;
        assert!(
            time::timeout(Duration::from_millis(150), delivered.recv())
                .await
                .is_err(),
            "known stale evidence must cancel in-flight delivery before awaiting the next GET"
        );
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        fresh_at.elapsed() < LEASE / 2,
        "withdrawal cannot be explained by lease expiry"
    );
    assert_eq!(
        manager.observation().state,
        ConnectionRuntimeState::Withdrawn
    );
    assert_eq!(
        manager.observation().reason,
        ConnectionRuntimeReason::InvalidConfiguration
    );
    {
        let view = controller.view.lock().unwrap();
        assert_eq!(view.snapshot.payload.revision, payload.revision + 1);
        assert!(view.snapshot.payload.paths.is_empty());
        assert_eq!(
            view.armed_gets,
            if evidence == Evidence::Publication {
                1
            } else {
                2
            }
        );
        assert_eq!(
            view.configuration_acks,
            if evidence == Evidence::Publication {
                1
            } else {
                2
            }
        );
        assert_eq!(
            view.connections_acks,
            if evidence == Evidence::ConnectionsAck {
                2
            } else {
                1
            }
        );
    }
    manager
        .withdraw(ConnectionRuntimeReason::ShuttingDown)
        .await;
    remote.shutdown().await;
}

#[tokio::test]
async fn configuration_ack_409_withdraws_before_stalled_retry_get() {
    known_stale_evidence_cancels_delivery(Evidence::ConfigurationAck, false).await;
}

#[tokio::test]
async fn connections_ack_409_withdraws_before_stalled_retry_get() {
    known_stale_evidence_cancels_delivery(Evidence::ConnectionsAck, false).await;
}

#[tokio::test]
async fn newer_identity_publication_withdraws_before_stalled_get() {
    known_stale_evidence_cancels_delivery(Evidence::Publication, false).await;
}

#[tokio::test]
async fn newer_publication_compares_installed_generation_even_when_saved_cache_is_already_newer() {
    known_stale_evidence_cancels_delivery(Evidence::Publication, true).await;
}
