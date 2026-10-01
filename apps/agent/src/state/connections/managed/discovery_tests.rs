use super::*;
use crate::managed_connections::tests::{fixture, snapshot};
use axum::response::IntoResponse;
use meshway_protocol::ConnectionTransport;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

struct Server {
    task: tokio::task::JoinHandle<()>,
    epoch: Arc<AtomicU32>,
    mode: Arc<AtomicU32>,
    begins: Arc<AtomicU32>,
    reads: Arc<AtomicU32>,
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
    let (e, m, b, r) = (epoch.clone(), mode.clone(), begins.clone(), reads.clone());
    let app = meshway_controller::router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let (epoch, mode, begins, reads) = (e.clone(), m.clone(), b.clone(), r.clone());
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
        },
        directory,
    )
}

async fn manager(lease: Duration) -> (Manager, Manager, String) {
    let (a, ak, b, bk) = fixture(ConnectionTransport::NativeUdp);
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
    let (mut left, mut right, peer) = manager(Duration::from_secs(10)).await;
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
    let (mut left, mut right, _) = manager(Duration::from_secs(10)).await;
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
    let (mut left, mut right, _) = manager(Duration::from_secs(10)).await;
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
    let (mut left, mut right, _) = manager(Duration::from_secs(10)).await;
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
