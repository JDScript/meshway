use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{Router, response::IntoResponse};
use meshway_agent::{Agent, compiled_connection_capabilities, connections_status};
use meshway_controller::{AppState, admin_token, router};
use serde::{Serialize, de::DeserializeOwned};

#[cfg(feature = "managed-connections")]
#[path = "connections_roundtrip/managed.rs"]
mod managed;

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

struct Server {
    base: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(app: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { base, task }
}

// Only admin test setup uses raw requests. Agent operations below use Agent's SDK path.
async fn admin_post<T: DeserializeOwned>(
    base: &str,
    admin: &str,
    path: &str,
    value: impl Serialize,
) -> T {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&value).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

async fn enrolled(server: &Server, admin: &str, path: &Path, name: &str) -> Agent {
    let token: meshway_protocol::CreateEnrollmentTokenResponse = admin_post(
        &server.base,
        admin,
        "/api/v1/enrollment-tokens",
        meshway_protocol::CreateEnrollmentTokenRequest {
            expires_in_seconds: 3600,
        },
    )
    .await;
    let mut agent = Agent::prepare_enrollment(path, &server.base, &token.token, name).unwrap();
    agent.enroll(&token.token).await.unwrap();
    agent
}

fn command(path: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .args(args)
        .arg("--state")
        .arg(path)
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_group_and_opt_in_run_keep_receipts_separate_and_publish_before_policy_pull() {
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let watched = calls.clone();
    let heartbeat_count = heartbeats.clone();
    let server = serve(router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let watched = watched.clone();
            let heartbeat_count = heartbeat_count.clone();
            async move {
                watched
                    .lock()
                    .unwrap()
                    .push(request.uri().path().to_owned());
                if request.uri().path() == "/api/v1/agents/heartbeat"
                    && heartbeat_count.fetch_add(1, Ordering::SeqCst) == 1
                {
                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                }
                next.run(request).await
            }
        },
    )))
    .await;
    let path = directory.path().join("enabled.json");
    let agent = enrolled(&server, &admin, &path, "enabled").await;
    drop(agent);
    let identity = std::fs::read(&path).unwrap();
    calls.lock().unwrap().clear();
    let output = command(&path, &["run", "--connections", "--interval-seconds", "1"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 401"));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["configuration"]["stored_revision"], 3);
    assert_eq!(receipt["connections"]["stored_revision"], 3);
    assert_eq!(receipt["connections"]["status"], "saved");
    assert_eq!(receipt["connections"]["applied"], false);
    assert_ne!(
        receipt["configuration"]["digest"],
        receipt["connections"]["digest"]
    );
    let requests = calls.lock().unwrap().clone();
    let at = |path| requests.iter().position(|request| request == path).unwrap();
    assert!(at("/api/v1/agent/connection-identity") < at("/api/v1/agents/configuration"));
    assert!(at("/api/v1/agents/configuration/ack") < at("/api/v1/agent/connections/ack"));
    assert_eq!(std::fs::read(&path).unwrap(), identity);
    for action in ["publish", "sync", "status"] {
        let output = command(&path, &["connections", action]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let public: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(public.get("private_key").is_none());
        if action != "publish" {
            assert_eq!(public["applied"], false);
        }
    }
    assert_eq!(
        connections_status(&path).unwrap().compiled_capabilities,
        compiled_connection_capabilities()
    );

    let path = directory.path().join("disabled.json");
    drop(enrolled(&server, &admin, &path, "disabled").await);
    calls.lock().unwrap().clear();
    heartbeats.store(0, Ordering::SeqCst);
    let output = command(&path, &["run", "--interval-seconds", "1"]);
    assert!(!output.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(receipt.get("connections").is_none());
    assert!(receipt.get("stored_revision").is_some());
    assert!(
        !directory
            .path()
            .join("disabled.json.connection-key.json")
            .exists()
    );
    assert!(
        !directory
            .path()
            .join("disabled.json.connections.json")
            .exists()
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|path| path.starts_with("/api/v1/agent/connection"))
    );
}

#[cfg(feature = "transport-lab")]
#[tokio::test]
async fn compiled_carrier_agents_save_real_node_scoped_reverse_dial_intent_and_fail_closed_withdrawal()
 {
    use meshway_protocol::{
        ConnectionCandidate, ConnectionEndpoint, ConnectionListener, ConnectionPlan,
        ConnectionTransport, ConnectionsSnapshot, SaveConnectionsRequest,
    };
    let directory = directory();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let server = serve(router(state)).await;
    let lax_path = directory.path().join("lax.json");
    let sha_path = directory.path().join("sha.json");
    let unrelated_path = directory.path().join("unrelated.json");
    let mut lax = enrolled(&server, &admin, &lax_path, "lax").await;
    let mut sha = enrolled(&server, &admin, &sha_path, "sha").await;
    let mut unrelated = enrolled(&server, &admin, &unrelated_path, "unrelated").await;
    let lax_public = lax.publish_connection_identity().await.unwrap();
    let sha_public = sha.publish_connection_identity().await.unwrap();
    let revision = unrelated
        .publish_connection_identity()
        .await
        .unwrap()
        .revision;
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let endpoint = ConnectionEndpoint {
        host: "127.0.0.1".into(),
        port,
    };
    let plan = ConnectionPlan {
        listeners: vec![ConnectionListener {
            id: "lax-vless".into(),
            node_id: lax_public.node_id.clone(),
            transport: ConnectionTransport::Vless,
            bind: format!("0.0.0.0:{port}"),
            advertised_endpoints: vec![endpoint.clone()],
            credential_ref: Some("lax-local-profile".into()),
        }],
        candidates: vec![ConnectionCandidate {
            id: "sha-dials-lax".into(),
            dialer_node_id: sha_public.node_id.clone(),
            listener_id: "lax-vless".into(),
            endpoint,
            priority: 10,
            credential_ref: Some("sha-local-profile".into()),
        }],
    };
    let saved: ConnectionsSnapshot = admin_post(
        &server.base,
        &admin,
        "/api/v1/connections",
        SaveConnectionsRequest {
            expected_revision: revision,
            configuration: plan,
        },
    )
    .await;
    assert!(saved.valid);
    for agent in [&mut lax, &mut sha, &mut unrelated] {
        let receipts = agent.sync_with_connections().await.unwrap();
        assert_eq!(receipts.configuration.stored_revision, saved.revision);
        assert_eq!(receipts.connections.stored_revision, saved.revision);
        assert!(receipts.connections.configuration_valid);
        assert!(!receipts.connections.applied);
    }
    #[derive(serde::Deserialize)]
    struct Saved {
        snapshot: meshway_protocol::AgentConnectionsSnapshot,
    }
    let cache = |name: &str| -> Saved {
        serde_json::from_slice(
            &std::fs::read(
                directory
                    .path()
                    .join(format!("{name}.json.connections.json")),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let lax_saved = cache("lax");
    let sha_saved = cache("sha");
    let unrelated_saved = cache("unrelated");
    assert_eq!(
        lax_saved.snapshot.payload.listeners[0]
            .credential_ref
            .as_deref(),
        Some("lax-local-profile")
    );
    assert_eq!(
        lax_saved.snapshot.payload.incoming[0].peer_node_id,
        sha_public.node_id
    );
    assert!(lax_saved.snapshot.payload.outgoing.is_empty());
    assert_eq!(
        sha_saved.snapshot.payload.outgoing[0]
            .credential_ref
            .as_deref(),
        Some("sha-local-profile")
    );
    assert_eq!(
        sha_saved.snapshot.payload.outgoing[0].peer_node_id,
        lax_public.node_id
    );
    assert!(sha_saved.snapshot.payload.listeners.is_empty());
    assert!(sha_saved.snapshot.payload.incoming.is_empty());
    assert!(unrelated_saved.snapshot.payload.peers.is_empty());
    assert!(unrelated_saved.snapshot.payload.listeners.is_empty());
    assert!(unrelated_saved.snapshot.payload.outgoing.is_empty());
    assert!(unrelated_saved.snapshot.payload.incoming.is_empty());
    let lax_bytes = std::fs::read(directory.path().join("lax.json.connections.json")).unwrap();
    let sha_bytes = std::fs::read(directory.path().join("sha.json.connections.json")).unwrap();
    assert!(!String::from_utf8_lossy(&lax_bytes).contains("sha-local-profile"));
    assert!(!String::from_utf8_lossy(&sha_bytes).contains("lax-local-profile"));
    // The configured socket remains available: this slice never starts listeners.
    let _listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    let _: meshway_protocol::RevokeNodeResponse = admin_post(
        &server.base,
        &admin,
        "/api/v1/nodes/revoke",
        meshway_protocol::RevokeNodeRequest {
            node_id: sha_public.node_id,
        },
    )
    .await;
    let receipt = lax.sync_connections().await.unwrap();
    assert!(!receipt.configuration_valid);
    let withdrawn = cache("lax");
    assert!(!withdrawn.snapshot.payload.valid);
    assert!(withdrawn.snapshot.payload.listeners.is_empty());
    assert!(withdrawn.snapshot.payload.incoming.is_empty());
    assert!(withdrawn.snapshot.payload.peers.is_empty());
}
