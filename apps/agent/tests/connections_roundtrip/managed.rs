use std::{
    fs::File,
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

use meshway_protocol::{
    ConnectionCandidate, ConnectionEndpoint, ConnectionListener, ConnectionPlan,
    ConnectionRuntimeState, ConnectionRuntimesSnapshot, ConnectionTransport, ConnectionsSnapshot,
    SaveConnectionsRequest,
};

use super::*;

#[path = "managed/discovery.rs"]
mod discovery;
#[path = "managed/latency.rs"]
mod latency;

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(path: &Path, log: &Path) -> Process {
    let file = File::create(log).unwrap();
    std::fs::set_permissions(log, std::fs::Permissions::from_mode(0o600)).unwrap();
    Process(
        Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .args([
                "run",
                "--apply-connections",
                "--interval-seconds",
                "1",
                "--connection-lease-seconds",
                "10",
                "--state",
            ])
            .arg(path)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap(),
    )
}

async fn observations(server: &Server, admin: &str) -> ConnectionRuntimesSnapshot {
    let response = reqwest::Client::new()
        .get(admin_url(&server.base, admin, "/connections/runtime").await)
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

async fn wait_neighbors(server: &Server, admin: &str, expected: bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let state = observations(server, admin).await;
            if state.nodes.len() == 2
                && state.nodes.iter().all(|node| {
                    node.state == ConnectionRuntimeState::Applied
                        && node.current
                        && !node.stale
                        && if expected {
                            node.peers.iter().any(|peer| {
                                peer.probe_fresh
                                    && peer.state
                                        == meshway_protocol::ConnectionNeighborState::Authenticated
                            })
                        } else {
                            node.peers.is_empty() && node.listeners.is_empty()
                        }
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("runtime reports did not converge");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_begin_response_pins_startup_cas_and_cannot_steal_new_owner() {
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let committed = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let begins = Arc::new(AtomicUsize::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let (committed_watch, release_watch, begins_watch, reads_watch) = (
        committed.clone(),
        release.clone(),
        begins.clone(),
        reads.clone(),
    );
    let server = serve(router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let (committed, release, begins, reads) = (
                committed_watch.clone(),
                release_watch.clone(),
                begins_watch.clone(),
                reads_watch.clone(),
            );
            async move {
                if request.method() == axum::http::Method::GET
                    && request.uri().path() == "/api/v1/agent/connections/runtime"
                {
                    reads.fetch_add(1, Ordering::SeqCst);
                }
                if request.uri().path() == "/api/v1/agent/connections/runtime/begin"
                    && begins.fetch_add(1, Ordering::SeqCst) == 0
                {
                    let response = next.run(request).await;
                    assert_eq!(response.status(), 200);
                    committed.notify_one();
                    release.notified().await;
                    return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                next.run(request).await
            }
        },
    )))
    .await;
    let path = directory.path().join("agent.json");
    drop(enrolled(&server, &admin, &path, "lost-begin").await);
    let log = directory.path().join("agent.log");
    let mut process = run(&path, &log);
    tokio::time::timeout(Duration::from_secs(5), committed.notified())
        .await
        .unwrap();
    #[derive(serde::Deserialize)]
    struct Credential {
        node_credential: String,
    }
    let credential: Credential = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let client = meshway_control_client::client::HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&credential.node_credential);
    let expected_epoch = client.get_connection_runtime_epoch().await.unwrap().epoch;
    assert_eq!(expected_epoch, 1);
    let newer = client
        .begin_connection_runtime(
            meshway_control_client::types::BeginConnectionRuntimeRequest {
                instance_id: meshway_protocol::local::credential().unwrap(),
                expected_epoch,
            },
        )
        .await
        .unwrap();
    assert_eq!(newer.epoch, 2);
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(begins.load(Ordering::SeqCst), 3);
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "old process must not refresh its CAS epoch"
    );
    let observed = observations(&server, &admin).await;
    assert_eq!(observed.nodes[0].epoch, 2);
    assert_eq!(observed.nodes[0].sequence, 0);
    assert_eq!(
        observed.nodes[0].state,
        ConnectionRuntimeState::AwaitingReport
    );
    assert!(
        std::fs::read_to_string(log)
            .unwrap()
            .contains("runtime lease superseded")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn superseded_runtime_epoch_is_fatal_and_revocation_closes_listener() {
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let server = serve(router(state)).await;
    let path = directory.path().join("agent.json");
    let mut agent = enrolled(&server, &admin, &path, "lease-owner").await;
    let identity = agent.publish_connection_identity().await.unwrap();
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: identity.revision,
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "only".into(),
                    node_id: identity.node_id.clone(),
                    transport: ConnectionTransport::NativeTcp,
                    bind: address.to_string(),
                    advertised_endpoints: vec![ConnectionEndpoint {
                        host: "localhost".into(),
                        port: address.port(),
                    }],
                    credential_ref: None,
                }],
                candidates: vec![],
            },
        },
    )
    .await;
    drop(agent);
    let log = directory.path().join("agent.log");
    let mut process = run(&path, &log);
    let wait_applied = || async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if observations(&server, &admin)
                    .await
                    .nodes
                    .iter()
                    .any(|node| node.state == ConnectionRuntimeState::Applied && node.current)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
    };
    wait_applied().await;
    #[derive(serde::Deserialize)]
    struct Credential {
        node_credential: String,
    }
    let credential: Credential = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let client = meshway_control_client::client::HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&credential.node_credential);
    let expected_epoch = client.get_connection_runtime_epoch().await.unwrap().epoch;
    client
        .begin_connection_runtime(
            meshway_control_client::types::BeginConnectionRuntimeRequest {
                instance_id: meshway_protocol::local::credential().unwrap(),
                expected_epoch,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("runtime lease superseded")
    );
    let mut process = run(&path, &log);
    wait_applied().await;
    let _: meshway_protocol::RevokeNodeResponse = admin_post(
        &server.base,
        &admin,
        "/nodes/revoke",
        meshway_protocol::RevokeNodeRequest {
            node_id: identity.node_id,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    let output = std::fs::read_to_string(&log).unwrap();
    assert!(output.contains("HTTP 403"));
    assert!(!output.contains(&credential.node_credential));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_controller_runtime_reports_durable_identity_removal_stalled_http_lease_and_ctrl_c()
{
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let stall = Arc::new(AtomicBool::new(false));
    let watched = stall.clone();
    let server = serve(router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let watched = watched.clone();
            async move {
                if request.uri().path() == "/api/v1/agents/heartbeat"
                    && watched.load(Ordering::SeqCst)
                {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                next.run(request).await
            }
        },
    )))
    .await;
    let lax_path = directory.path().join("lax.json");
    let sha_path = directory.path().join("sha.json");
    let mut lax = enrolled(&server, &admin, &lax_path, "lax").await;
    let mut sha = enrolled(&server, &admin, &sha_path, "sha").await;
    let lax_identity = lax.publish_connection_identity().await.unwrap();
    let sha_identity = sha.publish_connection_identity().await.unwrap();
    let lax_key = std::fs::read(directory.path().join("lax.json.connection-key.json")).unwrap();
    let sha_key = std::fs::read(directory.path().join("sha.json.connection-key.json")).unwrap();
    let lax_state = std::fs::read(&lax_path).unwrap();
    let sha_state = std::fs::read(&sha_path).unwrap();
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let endpoint = ConnectionEndpoint {
        host: "localhost".into(),
        port,
    };
    let plan = ConnectionPlan {
        listeners: vec![ConnectionListener {
            id: "lax-tcp".into(),
            node_id: lax_identity.node_id.clone(),
            transport: ConnectionTransport::NativeTcp,
            bind: format!("127.0.0.1:{port}"),
            advertised_endpoints: vec![endpoint.clone()],
            credential_ref: None,
        }],
        candidates: vec![ConnectionCandidate {
            id: "sha-dials-lax".into(),
            dialer_node_id: sha_identity.node_id.clone(),
            listener_id: "lax-tcp".into(),
            endpoint,
            priority: 0,
            credential_ref: None,
        }],
    };
    let saved: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: sha_identity.revision,
            configuration: plan.clone(),
        },
    )
    .await;
    assert!(saved.valid);
    drop(lax);
    drop(sha);
    let lax_log = directory.path().join("lax.log");
    let sha_log = directory.path().join("sha.log");
    let mut lax = run(&lax_path, &lax_log);
    let mut sha = run(&sha_path, &sha_log);
    wait_neighbors(&server, &admin, true).await;
    let observed = observations(&server, &admin).await;
    let lax_observed = observed
        .nodes
        .iter()
        .find(|node| node.node_id == lax_identity.node_id)
        .unwrap();
    assert_eq!(
        lax_observed.peers[0].initiator_node_id,
        sha_identity.node_id
    );
    assert_eq!(
        lax_observed.peers[0].listener_id.as_deref(),
        Some("lax-tcp")
    );
    assert!(lax_observed.peers[0].probe_rtt_ms.is_some());
    assert!(
        Agent::load(&lax_path).is_err(),
        "daemon must retain its state lock"
    );
    let ping = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &lax_path)
        .args(["ping", "sha", "--count", "1", "--timeout-seconds", "3"])
        .output()
        .unwrap();
    assert!(
        ping.status.success(),
        "{}",
        String::from_utf8_lossy(&ping.stderr)
    );
    let output = String::from_utf8(ping.stdout).unwrap();
    assert!(output.contains("authenticated neighbor probe / 认证邻接探测 (非ICMP，非mesh trace)"));
    assert!(output.contains(&sha_identity.node_id));
    assert!(output.contains("direction=accepted (peer initiated)"));
    assert!(output.contains("transport=native_tcp"));
    assert!(output.contains(" µs"));
    assert!(!output.contains("private_key"));
    let explicit = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &lax_path)
        .args(["ping", "lax", "--count", "1", "--state"])
        .arg(&sha_path)
        .output()
        .unwrap();
    assert!(
        explicit.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert!(
        String::from_utf8(explicit.stdout)
            .unwrap()
            .contains("direction=outgoing (locally initiated)")
    );
    let unknown = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &lax_path)
        .args(["ping", "unconfigured-nyc", "--count", "1"])
        .output()
        .unwrap();
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8(unknown.stderr)
            .unwrap()
            .contains("not a configured direct neighbor")
    );
    assert!(Agent::load(&lax_path).is_err());
    assert_eq!(std::fs::read(&lax_path).unwrap(), lax_state);
    assert_eq!(std::fs::read(&sha_path).unwrap(), sha_state);
    assert_eq!(
        std::fs::read(directory.path().join("lax.json.connection-key.json")).unwrap(),
        lax_key
    );
    assert_eq!(
        std::fs::read(directory.path().join("sha.json.connection-key.json")).unwrap(),
        sha_key
    );
    let empty: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: saved.revision,
            configuration: ConnectionPlan::default(),
        },
    )
    .await;
    wait_neighbors(&server, &admin, false).await;
    assert_eq!(
        meshway_agent::diagnostics::probe(&lax_path, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        meshway_agent::diagnostics::ProbeError::UnconfiguredPeer,
    );
    assert!(
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
    );
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: empty.revision,
            configuration: plan,
        },
    )
    .await;
    wait_neighbors(&server, &admin, true).await;
    stall.store(true, Ordering::SeqCst);
    // The in-flight HTTP heartbeat stalls for 60s. The independent 10s config
    // lease must still release the listener while the agent processes stay alive.
    tokio::time::timeout(Duration::from_secs(13), async {
        loop {
            if tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("HTTP stall extended the runtime lease");
    assert!(lax.0.try_wait().unwrap().is_none());
    assert!(sha.0.try_wait().unwrap().is_none());
    assert_eq!(
        meshway_agent::diagnostics::probe(&lax_path, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        meshway_agent::diagnostics::ProbeError::Unavailable,
        "IPC must answer unavailable independently of stalled controller HTTP",
    );
    for process in [&mut lax, &mut sha] {
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(process.0.id() as i32).unwrap(),
            rustix::process::Signal::INT,
        )
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        for process in [&mut lax, &mut sha] {
            loop {
                if let Some(status) = process.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    })
    .await
    .unwrap();
    for path in [&lax_path, &sha_path] {
        assert!(
            !meshway_agent::diagnostics::socket_path(path).exists(),
            "ctrl-C must clean up the private socket"
        );
    }
    for (path, bytes) in [(&lax_log, lax_key), (&sha_log, sha_key)] {
        let log = std::fs::read_to_string(path).unwrap();
        let private: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!log.contains(private["private_key"].as_str().unwrap()));
        assert!(log.contains("\"applied\":false"));
        assert!(log.contains("\"runtime\":"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_epoch_configuration_recovers_after_outage_backoff_exceeds_lease() {
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let mut server = serve(router(state.clone())).await;
    let controller_address = server.base.strip_prefix("http://").unwrap().to_owned();
    let lax_path = directory.path().join("lax.json");
    let sha_path = directory.path().join("sha.json");
    let mut lax = enrolled(&server, &admin, &lax_path, "lax").await;
    let mut sha = enrolled(&server, &admin, &sha_path, "sha").await;
    let lax_identity = lax.publish_connection_identity().await.unwrap();
    let sha_identity = sha.publish_connection_identity().await.unwrap();
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let endpoint = ConnectionEndpoint {
        host: "localhost".into(),
        port: address.port(),
    };
    let _: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/connections",
        SaveConnectionsRequest {
            expected_revision: sha_identity.revision,
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "lax-tcp".into(),
                    node_id: lax_identity.node_id,
                    transport: ConnectionTransport::NativeTcp,
                    bind: address.to_string(),
                    advertised_endpoints: vec![endpoint.clone()],
                    credential_ref: None,
                }],
                candidates: vec![ConnectionCandidate {
                    id: "sha-dials-lax".into(),
                    dialer_node_id: sha_identity.node_id,
                    listener_id: "lax-tcp".into(),
                    endpoint,
                    priority: 0,
                    credential_ref: None,
                }],
            },
        },
    )
    .await;
    drop(lax);
    drop(sha);
    let lax_log = directory.path().join("lax.log");
    let sha_log = directory.path().join("sha.log");
    let mut lax = run(&lax_path, &lax_log);
    let mut sha = run(&sha_path, &sha_log);
    wait_neighbors(&server, &admin, true).await;
    server.task.abort();
    let _ = (&mut server.task).await;
    tokio::time::timeout(Duration::from_secs(13), async {
        loop {
            if tokio::net::TcpListener::bind(address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("listener must close after the ten-second config lease");
    // Reach the real loop's 30-second outage backoff, beyond the ten-second
    // lease. A short outage cannot reproduce the reporter/reapply retry trap.
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            if [&lax_log, &sha_log].iter().all(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .contains("retrying in 30s")
            }) {
                break;
            }
            assert!(lax.0.try_wait().unwrap().is_none());
            assert!(sha.0.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("outage must exercise capped backoff while agents stay alive");
    let listener = tokio::net::TcpListener::bind(&controller_address)
        .await
        .unwrap();
    // Delay just the first recovery report through a new probe observation.
    // This makes the legitimate report fence deterministic instead of relying
    // on whether the generation's first probe beats the HTTP request scheduler.
    let first_report = Arc::new(AtomicBool::new(true));
    let app = router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let first_report = first_report.clone();
            async move {
                if request.method() == axum::http::Method::POST
                    && request.uri().path() == "/api/v1/agent/connections/runtime"
                    && first_report.swap(false, Ordering::SeqCst)
                {
                    tokio::time::sleep(Duration::from_secs(6)).await;
                    return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                next.run(request).await
            }
        },
    ));
    server.task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let before = observations(&server, &admin).await;
    let recovered = tokio::time::timeout(Duration::from_secs(70), async {
        loop {
            let after = observations(&server, &admin).await;
            if after.nodes.len() == 2
                && after.nodes.iter().all(|node| {
                    let old = before
                        .nodes
                        .iter()
                        .find(|old| old.node_id == node.node_id)
                        .unwrap();
                    node.state == ConnectionRuntimeState::Applied
                        && node.current
                        && !node.stale
                        && node.epoch == old.epoch
                        && node.revision == old.revision
                        && node.digest == old.digest
                        && node.sequence > old.sequence
                        && node.reported_at > old.reported_at
                        && node.peers.iter().any(|peer| {
                            peer.probe_fresh
                                && peer.state
                                    == meshway_protocol::ConnectionNeighborState::Authenticated
                        })
                })
            {
                break;
            }
            assert!(lax.0.try_wait().unwrap().is_none());
            assert!(sha.0.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let cancelled_long_retries: Vec<_> = [&lax_log, &sha_log]
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .filter(|line| {
                    line.contains("connection runtime observation changed")
                        && line.contains("retrying in 30s")
                })
                .count()
        })
        .collect();
    assert!(
        recovered.is_ok(),
        "fresh same-epoch config did not recover; fenced reports retaining 30s backoff: {cancelled_long_retries:?}"
    );
    assert!(
        tokio::net::TcpListener::bind(address).await.is_err(),
        "recovery must recreate the real listener"
    );
    assert_eq!(cancelled_long_retries, [0, 0]);
}
