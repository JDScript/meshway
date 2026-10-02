//! Real `meshway-agent run` entry point, real authenticated UDP probes and a
//! private Controller. No direct sampler launch or synthetic measurements.
use super::*;
use meshway_protocol::{LatencyHistoryResponse, QueryLatencyHistoryRequest};
use std::sync::atomic::AtomicU32;
use tokio::{
    sync::Notify,
    time::{sleep, timeout},
};

async fn history(server: &Server, admin: &str, source: &str, peer: &str) -> LatencyHistoryResponse {
    admin_post(
        &server.base,
        admin,
        "/latency/history/query",
        QueryLatencyHistoryRequest {
            source_node_id: source.into(),
            peer_node_id: peer.into(),
            from: chrono::Utc::now() - chrono::Duration::hours(1),
            until: chrono::Utc::now() + chrono::Duration::seconds(3),
            limit: 1000,
            after: None,
        },
    )
    .await
}

async fn stopped(process: &mut Process) {
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &process.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    timeout(Duration::from_secs(5), async {
        loop {
            if process.0.try_wait().unwrap().is_some() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normal_run_auto_history_single_sampler_restart_higher_revision_stall_and_lease() {
    let directory = directory();
    let data = directory.path().join("controller");
    let state = AppState::open(&data).unwrap();
    let admin = admin_token(&data).unwrap();
    let source_token: Arc<Mutex<Option<String>>> = Arc::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let reports = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicU32::new(0));
    let publication_stalled = Arc::new(Notify::new());
    let heartbeat_stalled = Arc::new(Notify::new());
    let (
        token_watch,
        attempts_watch,
        reports_watch,
        mode_watch,
        publication_watch,
        heartbeat_watch,
    ) = (
        source_token.clone(),
        attempts.clone(),
        reports.clone(),
        mode.clone(),
        publication_stalled.clone(),
        heartbeat_stalled.clone(),
    );
    let server = serve(router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let (token, attempts, reports, mode, publication, heartbeat) = (
                token_watch.clone(),
                attempts_watch.clone(),
                reports_watch.clone(),
                mode_watch.clone(),
                publication_watch.clone(),
                heartbeat_watch.clone(),
            );
            async move {
                let own = token.lock().unwrap().as_ref().is_some_and(|token| {
                    request
                        .headers()
                        .get("authorization")
                        .and_then(|h| h.to_str().ok())
                        == Some(format!("Bearer {token}").as_str())
                });
                if own {
                    if request.uri().path() == "/api/v1/agent/latency/samples" {
                        attempts.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.uri().path() == "/api/v1/agent/connections/runtime"
                        && request.method() == axum::http::Method::POST
                    {
                        reports.fetch_add(1, Ordering::SeqCst);
                    }
                    if mode.load(Ordering::SeqCst) == 1
                        && request.uri().path() == "/api/v1/agent/managed-transport-capabilities"
                    {
                        publication.notify_one();
                        std::future::pending::<()>().await;
                    }
                    if mode.load(Ordering::SeqCst) == 2
                        && request.uri().path() == "/api/v1/agents/heartbeat"
                    {
                        heartbeat.notify_one();
                        std::future::pending::<()>().await;
                    }
                }
                next.run(request).await
            }
        },
    )))
    .await;
    let a_path = directory.path().join("a.json");
    let b_path = directory.path().join("b.json");
    let mut a = enrolled(&server, &admin, &a_path, "latency-source").await;
    let mut b = enrolled(&server, &admin, &b_path, "latency-peer").await;
    let a_public = a.publish_connection_identity().await.unwrap();
    let b_public = b.publish_connection_identity().await.unwrap();
    let a_id = a_public.node_id;
    let b_id = b_public.node_id;
    drop(a);
    drop(b);
    let identity: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&a_path).unwrap()).unwrap();
    *source_token.lock().unwrap() = Some(identity["node_credential"].as_str().unwrap().to_owned());
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let response = reqwest::Client::new()
        .get(admin_url(&server.base, &admin, "/connections").await)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    let current: ConnectionsSnapshot =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let mut plan = ConnectionPlan {
        listeners: vec![ConnectionListener {
            id: "latency-listen".into(),
            node_id: a_id.clone(),
            transport: ConnectionTransport::NativeUdp,
            bind: format!("127.0.0.1:{port}"),
            advertised_endpoints: vec![ConnectionEndpoint {
                host: "127.0.0.1".into(),
                port,
            }],
            credential_ref: None,
        }],
        candidates: vec![ConnectionCandidate {
            id: "latency-dial".into(),
            dialer_node_id: b_id.clone(),
            listener_id: "latency-listen".into(),
            endpoint: ConnectionEndpoint {
                host: "127.0.0.1".into(),
                port,
            },
            priority: 0,
            credential_ref: None,
        }],
    };
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: current.revision,
            configuration: plan.clone(),
        },
    )
    .await;
    let mut a_process = run(&a_path, &directory.path().join("a.log"));
    let mut b_process = run(&b_path, &directory.path().join("b.log"));
    let first = timeout(Duration::from_secs(45), async {
        loop {
            let history = history(&server, &admin, &a_id, &b_id).await;
            if let Some(record) = history.records.first() {
                break record.clone();
            }
            assert!(
                a_process.0.try_wait().unwrap().is_none(),
                "Agent run exited: {}",
                std::fs::read_to_string(directory.path().join("a.log")).unwrap()
            );
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        first.sample.outcome,
        meshway_protocol::LatencyOutcome::Success
    );
    assert!(first.sample.rtt_us.unwrap() > 0);
    let count = attempts.load(Ordering::SeqCst);
    let old_reports = reports.load(Ordering::SeqCst);
    timeout(Duration::from_secs(8), async {
        while reports.load(Ordering::SeqCst) < old_reports + 3 {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        count,
        "heartbeats spawned extra immediate samplers"
    );
    assert!(
        history(&server, &admin, &a_id, &b_id)
            .await
            .records
            .iter()
            .all(|r| r.sample.sampler_generation == first.sample.sampler_generation)
    );
    // Real immutable retry via node credential, not another measurement.
    let body = meshway_protocol::ReportLatencySamplesRequest {
        samples: vec![first.sample.clone()],
    };
    let credential = source_token.lock().unwrap().as_ref().unwrap().clone();
    let result = reqwest::Client::new()
        .post(format!("{}/api/v1/agent/latency/samples", server.base))
        .bearer_auth(&credential)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&body).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 200);
    let receipt: meshway_protocol::ReportLatencySamplesResponse =
        serde_json::from_slice(&result.bytes().await.unwrap()).unwrap();
    assert_eq!(receipt.duplicates, 1);
    // Graceful shutdown joins the child; restarting run acquires a new epoch.
    stopped(&mut a_process).await;
    a_process = run(&a_path, &directory.path().join("a-restart.log"));
    let restarted = timeout(Duration::from_secs(45), async {
        loop {
            let h = history(&server, &admin, &a_id, &b_id).await;
            if let Some(r) = h
                .records
                .iter()
                .find(|r| r.sample.runtime_epoch > first.sample.runtime_epoch)
            {
                break r.clone();
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(
        restarted.sample.sampler_generation,
        first.sample.sampler_generation
    );
    // Keep the next publication pending after an actual higher-revision heartbeat.
    mode.store(1, Ordering::SeqCst);
    let response = reqwest::Client::new()
        .get(admin_url(&server.base, &admin, "/connections").await)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    let current: ConnectionsSnapshot =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    plan.candidates[0].priority = 1;
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: current.revision,
            configuration: plan,
        },
    )
    .await;
    timeout(Duration::from_secs(5), publication_stalled.notified())
        .await
        .unwrap();
    let count = attempts.load(Ordering::SeqCst);
    sleep(Duration::from_secs(31)).await;
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        count,
        "old samples continued during publication stall"
    );
    mode.store(0, Ordering::SeqCst);
    // The generated client's bounded wait recovers; then expire a live lease by
    // holding heartbeat indefinitely, independently of the sampler HTTP loop.
    let renewed = timeout(Duration::from_secs(45), async {
        loop {
            let h = history(&server, &admin, &a_id, &b_id).await;
            if let Some(r) = h
                .records
                .iter()
                .find(|r| r.sample.revision > restarted.sample.revision)
            {
                break r.clone();
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(renewed.sample.revision, restarted.sample.revision);
    mode.store(2, Ordering::SeqCst);
    timeout(Duration::from_secs(5), heartbeat_stalled.notified())
        .await
        .unwrap();
    let count = attempts.load(Ordering::SeqCst);
    sleep(Duration::from_secs(31)).await;
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        count,
        "expired lease kept producing samples"
    );
    mode.store(0, Ordering::SeqCst);
    stopped(&mut a_process).await;
    stopped(&mut b_process).await;
    let retained = history(&server, &admin, &a_id, &b_id).await;
    assert!(retained.records.iter().all(|r| r.historical));
}
