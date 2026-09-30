use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{Router, routing::post};
use meshway_agent::{
    Agent, AgentError,
    generated::{
        client::{EnrollAgentApiError, HttpClient},
        types::{AgentNodeResponse, EnrollAgentRequest, HeartbeatRequest},
    },
    status,
};
use meshway_controller::{AppState, admin_token, router};
use meshway_protocol::{CreateEnrollmentTokenResponse, local};
use serde_json::{Value, json};

fn tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
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

async fn token(base: &str, admin: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/enrollment-tokens"))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(r#"{"expires_in_seconds":3600}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice::<CreateEnrollmentTokenResponse>(&response.bytes().await.unwrap())
        .unwrap()
        .token
}

async fn revoke(base: &str, admin: &str, id: &str) {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/nodes/revoke"))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(json!({"node_id":id}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

fn enrollment(credential: &str, name: &str) -> EnrollAgentRequest {
    EnrollAgentRequest {
        node_credential: credential.to_owned(),
        name: name.to_owned(),
        agent_version: "0.1.0".to_owned(),
    }
}

#[tokio::test]
async fn generated_agent_operations_roundtrip_and_decode_scoped_errors() {
    let directory = tempdir();
    let server = serve(router(AppState::open(directory.path()).unwrap())).await;
    let admin = admin_token(directory.path()).unwrap();
    let token = token(&server.base, &admin).await;
    let credential = local::credential().unwrap();
    let client = HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&token);
    let error = client
        .enroll_agent(enrollment(&credential, " "))
        .await
        .unwrap_err();
    assert_eq!(error.api().unwrap().status, 400);
    assert!(matches!(
        error.api().unwrap().typed,
        Some(EnrollAgentApiError::Status400(_))
    ));
    let first = client
        .enroll_agent(enrollment(&credential, "nyc"))
        .await
        .unwrap();
    let retry = client
        .enroll_agent(enrollment(&credential, "nyc"))
        .await
        .unwrap();
    assert_eq!(first.node.id, retry.node.id);
    let heartbeat = HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&credential)
        .heartbeat_agent(HeartbeatRequest {
            agent_version: "0.2.0".into(),
        })
        .await
        .unwrap();
    assert_eq!(heartbeat.node.id, first.node.id);
    assert_eq!(heartbeat.node.agent_version, "0.2.0");
    assert!(heartbeat.node.last_seen >= first.node.last_seen);
    assert_eq!(
        client
            .heartbeat_agent(HeartbeatRequest {
                agent_version: "0.2.0".into()
            })
            .await
            .unwrap_err()
            .api()
            .unwrap()
            .status,
        401
    );
    revoke(&server.base, &admin, &first.node.id).await;
    assert_eq!(
        client
            .enroll_agent(enrollment(&credential, "nyc"))
            .await
            .unwrap_err()
            .api()
            .unwrap()
            .status,
        403
    );
}

#[tokio::test]
async fn generated_client_rejects_invalid_response_fields_and_dates_and_does_not_redirect_secrets()
{
    for body in [
        r#"{"node":{"id":"n","name":"nyc","agent_version":"0.1.0","last_seen":"not-a-date","registered_at":"2026-09-30T00:00:00Z","revoked_at":null}}"#,
        r#"{"node":{"id":7,"name":"nyc","agent_version":"0.1.0","last_seen":"2026-09-30T00:00:00Z","registered_at":"2026-09-30T00:00:00Z","revoked_at":null}}"#,
        r#"{"node":{"id":"n","name":"nyc","agent_version":"0.1.0","last_seen":"2026-09-30T00:00:00Z","registered_at":"2026-09-30T00:00:00Z"}}"#,
        r#"{"node":{"id":"n"}}"#,
    ] {
        let server = serve(
            Router::new()
                .route(
                    "/api/v1/agents/enroll",
                    post(move || async move { ([("content-type", "application/json")], body) }),
                )
                .route(
                    "/api/v1/agents/heartbeat",
                    post(move || async move { ([("content-type", "application/json")], body) }),
                ),
        )
        .await;
        let client = HttpClient::new()
            .with_base_url(&server.base)
            .with_api_key(local::credential().unwrap());
        let error = client
            .enroll_agent(enrollment(&local::credential().unwrap(), "nyc"))
            .await
            .unwrap_err();
        assert!(error.api().unwrap().parse_error.is_some());
        let error = client
            .heartbeat_agent(HeartbeatRequest {
                agent_version: "0.1.0".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.api().unwrap().status, 200);
        assert!(error.api().unwrap().parse_error.is_some());
        assert!(serde_json::from_str::<meshway_protocol::AgentNodeResponse>(body).is_err());
    }
    let leaked = Arc::new(AtomicUsize::new(0));
    let count = leaked.clone();
    let server = serve(
        Router::new()
            .route(
                "/api/v1/agents/enroll",
                post(|| async {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/redirected")],
                    )
                }),
            )
            .route(
                "/redirected",
                post(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "unexpected"
                    }
                }),
            ),
    )
    .await;
    let error = HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(local::credential().unwrap())
        .enroll_agent(enrollment(&local::credential().unwrap(), "nyc"))
        .await
        .unwrap_err();
    assert_eq!(error.api().unwrap().status, 307);
    assert_eq!(leaked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn required_nullable_revocation_accepts_explicit_null_and_preserves_presence_on_roundtrip() {
    for revoked_at in [Value::Null, json!("2026-09-30T01:00:00Z")] {
        let payload = json!({"node": {
            "id":"n", "name":"nyc", "agent_version":"0.1.0",
            "last_seen":"2026-09-30T00:00:00Z", "registered_at":"2026-09-30T00:00:00Z",
            "revoked_at":revoked_at,
        }});
        let body = payload.to_string();
        let server = serve(Router::new().route(
            "/api/v1/agents/heartbeat",
            post(move || {
                let body = body.clone();
                async move { ([("content-type", "application/json")], body) }
            }),
        ))
        .await;
        let response = HttpClient::new()
            .with_base_url(&server.base)
            .with_api_key(local::credential().unwrap())
            .heartbeat_agent(HeartbeatRequest {
                agent_version: "0.1.0".into(),
            })
            .await
            .unwrap();
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serialized["node"].get("revoked_at"),
            Some(&revoked_at),
            "None must serialize as an explicitly present null"
        );
        let roundtrip: AgentNodeResponse = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(roundtrip.node.revoked_at, response.node.revoked_at);
        let authoritative: meshway_protocol::AgentNodeResponse =
            serde_json::from_value(serialized).unwrap();
        assert_eq!(authoritative.node.revoked_at, response.node.revoked_at);
        let authoritative_serialized = serde_json::to_value(authoritative).unwrap();
        assert_eq!(
            authoritative_serialized["node"].get("revoked_at"),
            Some(&revoked_at)
        );
        let roundtrip: AgentNodeResponse =
            serde_json::from_value(authoritative_serialized).unwrap();
        assert_eq!(roundtrip.node.revoked_at, response.node.revoked_at);

        let mut absent = payload;
        absent["node"].as_object_mut().unwrap().remove("revoked_at");
        assert!(
            serde_json::from_value::<AgentNodeResponse>(absent.clone())
                .unwrap_err()
                .to_string()
                .contains("missing field `revoked_at`")
        );
        assert!(
            serde_json::from_value::<meshway_protocol::AgentNodeResponse>(absent)
                .unwrap_err()
                .to_string()
                .contains("missing field `revoked_at`")
        );
    }
}

#[tokio::test]
async fn locally_pending_enrollment_recovers_after_lost_response_without_duplicate_identity() {
    let directory = tempdir();
    let server = serve(router(
        AppState::open(directory.path().join("controller")).unwrap(),
    ))
    .await;
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let token = token(&server.base, &admin).await;
    let path = directory.path().join("agent.json");
    let agent = Agent::prepare_enrollment(&path, &server.base, &token, "recoverable").unwrap();
    assert_eq!(agent.status().enrollment, "pending");
    assert!(Agent::load(&path).is_err(), "exclusive state lock");
    let persisted = local::read_private(&path).unwrap();
    assert!(
        !persisted.contains(&token),
        "raw enrollment token must not be persisted"
    );
    let identity: Value = serde_json::from_str(&persisted).unwrap();
    let credential = identity["node_credential"].as_str().unwrap();
    // Server commits; local agent never receives/persists the response.
    let committed = HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&token)
        .enroll_agent(enrollment(credential, "recoverable"))
        .await
        .unwrap();
    drop(agent);
    assert_eq!(status(&path).unwrap().enrollment, "pending");
    let mut recovered = Agent::load(&path).unwrap();
    let heartbeat = recovered.heartbeat().await.unwrap();
    assert_eq!(heartbeat.node.id, committed.node.id);
    assert_eq!(
        status(&path).unwrap().node_id.as_deref(),
        Some(committed.node.id.as_str())
    );
    drop(recovered);
    let mut retry =
        Agent::prepare_enrollment(&path, &format!("{}/", server.base), &token, "recoverable")
            .unwrap();
    assert_eq!(
        retry.enroll(&token).await.unwrap().node.id,
        committed.node.id
    );
    drop(retry);
    let before = std::fs::read(&path).unwrap();
    assert!(
        Agent::prepare_enrollment(
            &path,
            &server.base,
            &local::credential().unwrap(),
            "recoverable"
        )
        .is_err()
    );
    assert!(Agent::prepare_enrollment(&path, &server.base, &token, "different").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let redacted = serde_json::to_string(&status(&path).unwrap()).unwrap();
    assert!(!redacted.contains(credential));
    assert!(!redacted.contains(&token));
    revoke(&server.base, &admin, &committed.node.id).await;
    assert!(matches!(
        Agent::load(&path).unwrap().heartbeat().await,
        Err(AgentError::Http {
            status: 403,
            retryable: false
        })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn state_files_are_private_atomic_and_never_clobber_unrelated_data() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let directory = tempdir();
    let path = directory.path().join("agent.json");
    let token = local::credential().unwrap();
    let prepare = || Agent::prepare_enrollment(&path, "http://127.0.0.1:1", &token, "local");
    let agent = prepare().unwrap();
    let initial = std::fs::read(&path).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    drop(agent);
    drop(prepare().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), initial);
    local::atomic_write(&path, b"unrelated", true).unwrap();
    assert!(prepare().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated");
    assert!(local::atomic_write(&path, b"must not overwrite", false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated");
    local::atomic_write(&path, &initial, true).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(prepare().is_err());
    assert!(local::atomic_write(&path, b"must not overwrite", true).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), initial);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let linked = directory.path().join("linked.json");
    std::fs::hard_link(&path, &linked).unwrap();
    assert!(prepare().is_err());
    std::fs::remove_file(&linked).unwrap();
    std::fs::rename(&path, &linked).unwrap();
    symlink(&linked, &path).unwrap();
    assert!(prepare().is_err());
    assert_eq!(std::fs::read(&linked).unwrap(), initial);
}

fn command(arguments: &[&str], path: &Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .args(arguments)
        .arg("--state")
        .arg(path)
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_enroll_heartbeat_status_retry_and_run_auth_failure() {
    let directory = tempdir();
    let server = serve(router(
        AppState::open(directory.path().join("controller")).unwrap(),
    ))
    .await;
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let token = token(&server.base, &admin).await;
    let path = directory.path().join("agent.json");
    let mut id = None;
    for arguments in [
        vec![
            "enroll",
            "--controller",
            &server.base,
            "--token",
            &token,
            "--name",
            "cli",
        ],
        vec!["heartbeat"],
        vec![
            "enroll",
            "--controller",
            &server.base,
            "--token",
            &token,
            "--name",
            "cli",
        ],
    ] {
        let output = command(&arguments, &path);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: AgentNodeResponse = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(id) = &id {
            assert_eq!(&response.node.id, id);
        }
        id = Some(response.node.id);
        assert_eq!(response.node.name, "cli");
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&token));
    }
    let output = command(&["status"], &path);
    assert!(output.status.success());
    let redacted: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(redacted["enrollment"], "enrolled");
    assert!(redacted.get("node_credential").is_none());
    revoke(&server.base, &admin, &id.unwrap()).await;
    let started = std::time::Instant::now();
    let output = command(&["run", "--interval-seconds", "15"], &path);
    assert!(!output.status.success());
    assert!(
        started.elapsed().as_secs() < 5,
        "run must heartbeat immediately and terminate on revocation"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 403"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_backs_off_transient_failures_resets_after_success_and_stops_on_401() {
    let directory = tempdir();
    let state = AppState::open(directory.path().join("controller")).unwrap();
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let attempts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = attempts.clone();
    let app = router(state).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let captured = captured.clone();
            async move {
                if request.uri().path() == "/api/v1/agents/heartbeat" {
                    let attempt = {
                        let mut times = captured.lock().unwrap();
                        times.push(std::time::Instant::now());
                        times.len()
                    };
                    use axum::response::IntoResponse;
                    match attempt {
                        1 | 2 | 4 => {
                            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        5 => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
                        _ => {}
                    }
                }
                next.run(request).await
            }
        },
    ));
    let server = serve(app).await;
    let token = token(&server.base, &admin).await;
    let path = directory.path().join("agent.json");
    let mut agent = Agent::prepare_enrollment(&path, &server.base, &token, "backoff").unwrap();
    agent.enroll(&token).await.unwrap();
    drop(agent);
    let output = command(&["run", "--interval-seconds", "1"], &path);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("retrying in 1s"));
    assert!(stderr.contains("retrying in 2s"));
    assert!(!stderr.contains("retrying in 4s"));
    assert!(stderr.contains("HTTP 401"));
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 1);
    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts.len(), 5);
    for (window, minimum_ms) in attempts.windows(2).zip([900, 1900, 900, 900]) {
        assert!(window[1].duration_since(window[0]).as_millis() >= minimum_ms);
    }
}

#[tokio::test]
async fn enrollment_never_overwrites_state_replaced_while_request_is_in_flight() {
    let directory = tempdir();
    let server = serve(router(
        AppState::open(directory.path().join("controller")).unwrap(),
    ))
    .await;
    let admin = admin_token(directory.path().join("controller")).unwrap();
    let token = token(&server.base, &admin).await;
    let path = directory.path().join("agent.json");
    let mut agent = Agent::prepare_enrollment(&path, &server.base, &token, "pending").unwrap();
    let backup = local::read_private(&path).unwrap();
    // Simulate an out-of-band restore which does not honor the sidecar lock.
    local::atomic_write(&path, b"unrelated user data", true).unwrap();
    assert!(agent.enroll(&token).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated user data");
    drop(agent);
    local::atomic_write(&path, backup.as_bytes(), true).unwrap();
    let recovered = Agent::load(&path).unwrap().heartbeat().await.unwrap();
    assert_eq!(recovered.node.name, "pending");
    assert_eq!(status(&path).unwrap().enrollment, "enrolled");
}
