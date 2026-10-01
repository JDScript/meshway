use super::*;
use axum::{
    extract::{Request, State as HttpState},
    http::Method,
    response::IntoResponse,
};
use meshway_protocol::{
    BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse, ConnectionEndpoint,
    ConnectionTransport, GetConnectionRuntimeEpochResponse, OutgoingConnectionCandidate,
    ReportConnectionRuntimeRequest, ReportConnectionRuntimeResponse,
};
use std::sync::atomic::AtomicUsize;
use tokio::{sync::Notify, time};

struct Controller {
    current: Mutex<AgentDataplaneSnapshot>,
    recovered: AgentDataplaneSnapshot,
    gets: AtomicUsize,
    acks: AtomicUsize,
    applied: Notify,
}

async fn operation(
    HttpState(state): HttpState<Arc<Controller>>,
    request: Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 8192).await.unwrap();
    let path = parts.uri.path();
    if path == "/api/v1/agent/dataplane" {
        let previous = state.gets.fetch_add(1, Ordering::SeqCst);
        if previous == 1 {
            // The first DNS failure must have retried the actual GET, without
            // ACKing/applying the old cached bundle in between.
            assert_eq!(state.acks.load(Ordering::SeqCst), 0);
            *state.current.lock().unwrap() = state.recovered.clone();
        }
    }
    let snapshot = state.current.lock().unwrap().clone();
    let bytes = match (parts.method, path) {
        (Method::GET, "/api/v1/agent/connections/runtime") => {
            serde_json::to_vec(&GetConnectionRuntimeEpochResponse { epoch: 0 }).unwrap()
        }
        (Method::POST, "/api/v1/agent/connections/runtime/begin") => {
            let request: BeginConnectionRuntimeRequest = serde_json::from_slice(&body).unwrap();
            assert_eq!(request.expected_epoch, 0);
            serde_json::to_vec(&BeginConnectionRuntimeResponse { epoch: 1 }).unwrap()
        }
        (Method::POST, "/api/v1/agent/connections/runtime") => {
            let request: ReportConnectionRuntimeRequest = serde_json::from_slice(&body).unwrap();
            assert_eq!(request.revision, state.recovered.payload.revision);
            assert_eq!(
                request.digest,
                connection_digest(&state.recovered.payload.connections).unwrap()
            );
            if request.state == ConnectionRuntimeState::Applied {
                state.applied.notify_one();
            }
            serde_json::to_vec(&ReportConnectionRuntimeResponse {
                epoch: request.epoch,
                sequence: request.sequence,
                reported_at: chrono::Utc::now(),
            })
            .unwrap()
        }
        (_, path) => {
            if path.ends_with("/ack") {
                state.acks.fetch_add(1, Ordering::SeqCst);
            }
            super::revocation::success(path, &body, &snapshot)
        }
    };
    ([("content-type", "application/json")], bytes).into_response()
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn startup_dns_failure_retries_fresh_bundle_without_ack_or_new_identity() {
    let dir = directory();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (mut agent, mut payload) = enrolled(dir.path(), &base);
    let target = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    payload
        .connections
        .outgoing
        .push(OutgoingConnectionCandidate {
            id: "dns-startup".into(),
            listener_id: "peer-listener".into(),
            peer_node_id: payload.configuration.peers[0].id.clone(),
            transport: ConnectionTransport::NativeUdp,
            endpoint: ConnectionEndpoint {
                host: "meshway-peer-not-started.invalid".into(),
                port: target.local_addr().unwrap().port(),
            },
            priority: 0,
            credential_ref: None,
        });
    let old = snapshot(payload.clone());
    payload.revision += 1;
    payload.configuration.revision += 1;
    payload.connections.revision += 1;
    payload.connections.outgoing[0].endpoint.host = "127.0.0.1".into();
    let recovered = snapshot(payload.clone());
    let control = Arc::new(Controller {
        current: Mutex::new(old.clone()),
        recovered: recovered.clone(),
        gets: AtomicUsize::new(0),
        acks: AtomicUsize::new(0),
        applied: Notify::new(),
    });
    let app = axum::Router::new()
        .fallback(operation)
        .with_state(control.clone());
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap()
    }));
    let mut manager = Manager::new_dataplane(
        None,
        Duration::from_secs(10),
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        base,
        false,
    );
    // Before any device exists, startup resolution is retryable and cannot
    // create a runtime or proceed to Linux TUN configuration.
    let error = manager.prepare_dataplane_sink(&old).await.unwrap_err();
    assert!(matches!(error, AgentError::DataplaneResolveFailed));
    assert!(error.retryable());
    assert_eq!(
        manager.observation().reason,
        ConnectionRuntimeReason::ResolveFailed
    );
    let (device, _send, _receive) = MockDevice::new();
    manager.mock_tun(Tun::mock(
        device.clone(),
        &PacketPlan::sink(&old.payload, 1000).unwrap(),
    ));
    let identity_bytes = local::read_private(&agent.path).unwrap();
    let key_bytes = local::read_private(&connection_key::path(&agent.path)).unwrap();
    let mut dataplane = Some(State::load(&mut agent).unwrap());
    {
        let run = agent.managed_loop(&mut manager, Duration::from_secs(1), &mut dataplane);
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => panic!("startup DNS failure terminated the managed process: {result:?}"),
            result = time::timeout(Duration::from_secs(8), control.applied.notified()) => result.expect("fresh DNS/configuration recovery must apply within the retry budget"),
        }
    }
    assert!(
        control.gets.load(Ordering::SeqCst) >= 3,
        "retry plus a fresh fetch while holding the runtime epoch"
    );
    assert_eq!(manager.observation().state, ConnectionRuntimeState::Applied);
    assert_eq!(
        manager.snapshot.as_ref().unwrap().payload.revision,
        recovered.payload.revision
    );
    assert_eq!(
        dataplane
            .as_ref()
            .unwrap()
            .cache
            .as_ref()
            .unwrap()
            .snapshot
            .digest,
        recovered.digest
    );
    assert_eq!(local::read_private(&agent.path).unwrap(), identity_bytes);
    assert_eq!(
        local::read_private(&connection_key::path(&agent.path)).unwrap(),
        key_bytes
    );
    assert!(
        Arc::strong_count(&device) >= 2,
        "same owned route sink survives DNS retry"
    );
    manager
        .withdraw(ConnectionRuntimeReason::ShuttingDown)
        .await;
}
