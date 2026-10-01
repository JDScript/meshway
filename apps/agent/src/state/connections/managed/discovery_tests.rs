use super::*;
use crate::managed_connections::tests::{fixture, snapshot};
use axum::response::IntoResponse;
use meshway_protocol::ConnectionTransport;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

struct Server {
    task: tokio::task::JoinHandle<()>,
    epoch: Arc<AtomicU32>,
    mode: Arc<AtomicU32>,
    begins: Arc<AtomicU32>,
    reads: Arc<AtomicU32>,
    projection: Arc<Mutex<Option<meshway_protocol::AgentConnectionsSnapshot>>>,
    projection_reads: Arc<AtomicU32>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn setup() -> (Agent, RuntimeLease, Server, tempfile::TempDir) {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let controller = directory.path().join("controller");
    let state = meshway_controller::AppState::open(&controller).unwrap();
    let admin = meshway_controller::admin_token(&controller).unwrap();
    let epoch = Arc::new(AtomicU32::new(1));
    let mode = Arc::new(AtomicU32::new(409));
    let begins = Arc::new(AtomicU32::new(0));
    let reads = Arc::new(AtomicU32::new(0));
    let projection: Arc<Mutex<Option<meshway_protocol::AgentConnectionsSnapshot>>> = Arc::default();
    let projection_reads = Arc::new(AtomicU32::new(0));
    let (projection_watch, projection_reads_watch) = (projection.clone(), projection_reads.clone());
    let (e, m, b, r) = (epoch.clone(), mode.clone(), begins.clone(), reads.clone());
    let app = meshway_controller::router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let (epoch, mode, begins, reads) = (e.clone(), m.clone(), b.clone(), r.clone());
            let projection = projection_watch.clone();
            let projection_reads = projection_reads_watch.clone();
            async move {
                if request.uri().path() == "/api/v1/agent/connections/runtime/begin" {
                    begins.fetch_add(1, Ordering::SeqCst);
                }
                if request.uri().path() == "/api/v1/agent/connections/runtime"
                    && request.method() == axum::http::Method::GET
                {
                    reads.fetch_add(1, Ordering::SeqCst);
                    if mode.load(Ordering::SeqCst) == 408 {
                        time::sleep(Duration::from_secs(60)).await;
                    }
                    return axum::Json(GetConnectionRuntimeEpochResponse {
                        epoch: epoch.load(Ordering::SeqCst),
                    })
                    .into_response();
                }
                if request.uri().path() == "/api/v1/agent/connections/discovery" {
                    if matches!(mode.load(Ordering::SeqCst), 422 | 423 | 403) {
                        return axum::http::StatusCode::UNPROCESSABLE_ENTITY.into_response();
                    }
                    if mode.load(Ordering::SeqCst) != 200 {
                        return axum::http::StatusCode::CONFLICT.into_response();
                    }
                    let bytes = axum::body::to_bytes(request.into_body(), 256 * 1024)
                        .await
                        .unwrap();
                    let report: ReportEndpointDiscoveryRequest =
                        serde_json::from_slice(&bytes).unwrap();
                    return axum::Json(ReportEndpointDiscoveryResponse {
                        epoch: report.epoch,
                        sequence: report.sequence,
                        reported_at: chrono::Utc::now(),
                    })
                    .into_response();
                }
                if request.uri().path() == "/api/v1/agent/connections"
                    && request.method() == axum::http::Method::GET
                {
                    projection_reads.fetch_add(1, Ordering::SeqCst);
                    if mode.load(Ordering::SeqCst) == 423 {
                        time::sleep(Duration::from_secs(60)).await;
                    }
                    if mode.load(Ordering::SeqCst) == 403 {
                        return axum::http::StatusCode::FORBIDDEN.into_response();
                    }
                    let projection = projection.lock().unwrap().clone();
                    if let Some(projection) = projection {
                        return axum::Json(projection).into_response();
                    }
                }
                next.run(request).await
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/enrollment-tokens"))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&meshway_protocol::CreateEnrollmentTokenRequest {
                expires_in_seconds: 3600,
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    let token: meshway_protocol::CreateEnrollmentTokenResponse =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let mut agent = Agent::prepare_enrollment(
        directory.path().join("agent.json"),
        &base,
        &token.token,
        "diagnostic",
    )
    .unwrap();
    agent.enroll(&token.token).await.unwrap();
    agent.publish_connection_identity().await.unwrap();
    let mut lease = RuntimeLease::new().unwrap();
    let response = agent
        .runtime_client()
        .begin_connection_runtime(
            convert(BeginConnectionRuntimeRequest {
                instance_id: lease.instance_id.clone(),
                expected_epoch: 0,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let response: BeginConnectionRuntimeResponse = convert(response).unwrap();
    lease.epoch = Some(response.epoch);
    (
        agent,
        lease,
        Server {
            task,
            epoch,
            mode,
            begins,
            reads,
            projection,
            projection_reads,
        },
        directory,
    )
}

async fn manager(agent: &Agent, lease: Duration) -> (Manager, Manager, String) {
    let (mut a, _, mut b, bk) = fixture(ConnectionTransport::NativeUdp);
    let own_id = agent.identity.node_id.as_ref().unwrap().clone();
    let own_key = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .public_key()
        .unwrap()
        .to_owned();
    let ak = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .application_key(&agent.path, &agent.identity)
        .unwrap();
    a.node_id = own_id.clone();
    a.public_key = Some(own_key.clone());
    for listener in &mut a.listeners {
        listener.node_id = own_id.clone();
    }
    for peer in &mut b.peers {
        peer.node_id = own_id.clone();
        peer.public_key = own_key.clone();
    }
    for candidate in &mut b.outgoing {
        candidate.peer_node_id = own_id.clone();
    }
    let peer = b.node_id.clone();
    let mut left = Manager::new(None, lease);
    let mut right = Manager::new(None, lease);
    left.reconcile(snapshot(a), ak, Instant::now()).await;
    right.reconcile(snapshot(b), bk, Instant::now()).await;
    time::timeout(Duration::from_secs(4), async {
        loop {
            if left
                .prober()
                .probe_neighbor_observed(crate::diagnostics::ProbeRequest {
                    peer: peer.clone(),
                    timeout_ms: 1000,
                })
                .await
                .is_ok()
            {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (left, right, peer)
}

#[tokio::test]
async fn same_epoch_watermark_conflict_preserves_noise_and_next_report_recovers() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, peer) = manager(&agent, Duration::from_secs(10)).await;
    agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("conflict emitted receipt")
        })
        .await
        .unwrap();
    assert_eq!(server.reads.load(Ordering::SeqCst), 1);
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    left.prober()
        .probe_neighbor_observed(crate::diagnostics::ProbeRequest {
            peer,
            timeout_ms: 1000,
        })
        .await
        .unwrap();
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    server.mode.store(200, Ordering::SeqCst);
    let mut receipt = String::new();
    agent
        .report_discovery(
            &mut left,
            &mut lease,
            DiscoveryReportMode::Normal,
            |value| receipt.push_str(value),
        )
        .await
        .unwrap();
    assert!(receipt.contains("endpoint_discovery"));
    assert_eq!(lease.discovery_sequence, 2);
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn confirmed_new_epoch_is_fatal_without_begin_or_reacquisition() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    server.epoch.store(2, Ordering::SeqCst);
    let error = agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("conflict emitted receipt")
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("superseded"));
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Withdrawn
    );
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    assert_eq!(lease.epoch, Some(1));
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn successful_shutdown_report_never_touches_receipt_sink() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    left.withdraw(Reason::ShuttingDown).await;
    server.mode.store(200, Ordering::SeqCst);
    time::timeout(
        Duration::from_secs(3),
        agent.report_discovery(&mut left, &mut lease, DiscoveryReportMode::Shutdown, |_| {
            panic!("shutdown touched potentially full stdout sink")
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(lease.discovery_sequence, 1);
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn stalled_epoch_confirmation_is_bounded_and_does_not_renew_runtime() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    server.mode.store(408, Ordering::SeqCst);
    let started = Instant::now();
    agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("conflict emitted receipt")
        })
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert_eq!(server.reads.load(Ordering::SeqCst), 1);
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn diagnostic_422_same_source_preserves_noise_and_real_dns_same_revision() {
    use crate::managed_dns::{Guard, Server as DnsServer, Zone};
    use hickory_proto::{
        op::{Message, MessageType, OpCode, Query},
        rr::{Name, RecordType},
    };
    let (mut agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, peer) = manager(&agent, Duration::from_secs(10)).await;
    let source = left.snapshot.as_ref().unwrap().clone();
    *server.projection.lock().unwrap() = Some(source.clone());
    server.mode.store(422, Ordering::SeqCst);
    let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let listen = reserved.local_addr().unwrap();
    drop(reserved);
    let dns = DnsServer::bind(listen).await.unwrap();
    let guard = Guard::default();
    guard.listener_bound(listen, agent.identity.node_id.clone());
    let owner = Name::from_ascii("diagnostic.meshway.test.").unwrap();
    assert!(guard.publish(
        Zone {
            name: Name::from_ascii("meshway.test.").unwrap(),
            ttl: 30,
            forward: std::collections::BTreeMap::from([(
                owner.clone(),
                ("100.64.0.1".parse().unwrap(), "fd00::1".parse().unwrap())
            )]),
            reverse: Default::default(),
        },
        Instant::now() + Duration::from_secs(10),
        source.payload.revision
    ));
    agent.dns_guard = Some(guard.clone());
    let dns_guard = guard.clone();
    let dns_task = tokio::spawn(async move {
        dns.serve(dns_guard).await.unwrap();
    });
    let before = guard.status();
    for _ in 0..2 {
        agent
            .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
                panic!("rejected diagnostic emitted receipt")
            })
            .await
            .unwrap();
        left.prober()
            .probe_neighbor_observed(crate::diagnostics::ProbeRequest {
                peer: peer.clone(),
                timeout_ms: 1000,
            })
            .await
            .unwrap();
        let after = guard.status();
        assert_eq!(after.state, crate::diagnostics::dns::State::Active);
        assert_eq!(after.revision_floor, before.revision_floor);
        assert_eq!(after.active_revision, before.active_revision);
        assert_eq!(after.generation, before.generation);
        assert!(after.lease_remaining_ms <= before.lease_remaining_ms);
        let mut query = Message::new(123, MessageType::Query, OpCode::Query);
        query.add_query(Query::query(owner.clone(), RecordType::A));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(&query.to_vec().unwrap(), listen)
            .await
            .unwrap();
        let mut bytes = [0_u8; 512];
        let (len, sender) = time::timeout(Duration::from_secs(1), client.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sender, listen);
        let answer = Message::from_vec(&bytes[..len]).unwrap();
        assert_eq!(answer.answers.len(), 1);
    }
    assert_eq!(server.projection_reads.load(Ordering::SeqCst), 2);
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    assert_eq!(left.snapshot.as_ref().unwrap().digest, source.digest);
    guard.shutdown();
    dns_task.abort();
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn diagnostic_422_confirmed_source_change_withdraws_and_requests_fresh_pull() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    let previous = left.snapshot.as_ref().unwrap().clone();
    let mut next = previous.payload.clone();
    next.revision += 1;
    *server.projection.lock().unwrap() = Some(snapshot(next));
    server.mode.store(422, Ordering::SeqCst);
    let error = agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("rejected diagnostic emitted receipt")
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Http {
            status: 422,
            retryable: true
        }
    ));
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Withdrawn
    );
    assert_eq!(left.observation().reason, Reason::InvalidConfiguration);
    // Confirmation is evidence for teardown/pull, never an ACK or new lease.
    assert_eq!(left.snapshot.as_ref().unwrap().digest, previous.digest);
    assert_eq!(server.projection_reads.load(Ordering::SeqCst), 1);
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn diagnostic_422_invalid_confirmation_is_not_source_change_and_403_stays_fatal() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    let mut invalid = left.snapshot.as_ref().unwrap().clone();
    invalid.payload.revision += 1;
    invalid.digest = "f".repeat(64);
    *server.projection.lock().unwrap() = Some(invalid);
    server.mode.store(422, Ordering::SeqCst);
    agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("rejected diagnostic emitted receipt")
        })
        .await
        .unwrap();
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    server.mode.store(403, Ordering::SeqCst);
    let error = agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("rejected diagnostic emitted receipt")
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Http {
            status: 403,
            retryable: false
        }
    ));
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
}

#[tokio::test]
async fn diagnostic_422_confirmation_shares_budget_and_watchdog_still_expires() {
    let (agent, mut lease, server, _directory) = setup().await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_secs(10)).await;
    server.mode.store(423, Ordering::SeqCst);
    let started = Instant::now();
    agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("rejected diagnostic emitted receipt")
        })
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert_eq!(
        left.observation().state,
        meshway_protocol::ConnectionRuntimeState::Applied
    );
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
    let (mut left, mut right, _) = manager(&agent, Duration::from_millis(400)).await;
    let bound: std::net::SocketAddr = left.snapshot.as_ref().unwrap().payload.listeners[0]
        .bind
        .parse()
        .unwrap();
    let started = Instant::now();
    agent
        .report_discovery(&mut left, &mut lease, DiscoveryReportMode::Normal, |_| {
            panic!("rejected diagnostic emitted receipt")
        })
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    time::timeout(Duration::from_secs(1), async {
        loop {
            if left.observation().state == meshway_protocol::ConnectionRuntimeState::Withdrawn
                && tokio::net::UdpSocket::bind(bound).await.is_ok()
            {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(server.begins.load(Ordering::SeqCst), 1);
    left.withdraw(Reason::ShuttingDown).await;
    right.withdraw(Reason::ShuttingDown).await;
}
