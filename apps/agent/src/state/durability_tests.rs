use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::Router;
use meshway_controller::{AppState, admin_token, router};
use meshway_protocol::CreateEnrollmentTokenResponse;
use serde_json::{Value, json};

use super::{Agent, AgentError, Identity, local};
use local::test_faults::DirectorySyncFailure;

fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

struct Server {
    base: String,
    admin: String,
    acks: Arc<AtomicUsize>,
    heartbeats: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(path: &std::path::Path) -> Self {
        let state = AppState::open(path).unwrap();
        let admin = admin_token(path).unwrap();
        let acks = Arc::new(AtomicUsize::new(0));
        let heartbeats = Arc::new(AtomicUsize::new(0));
        let observed_acks = acks.clone();
        let observed_heartbeats = heartbeats.clone();
        let app: Router = router(state).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let acks = observed_acks.clone();
                let heartbeats = observed_heartbeats.clone();
                async move {
                    match request.uri().path() {
                        "/api/v1/agents/configuration/ack" => {
                            acks.fetch_add(1, Ordering::SeqCst);
                        }
                        "/api/v1/agents/heartbeat" => {
                            heartbeats.fetch_add(1, Ordering::SeqCst);
                        }
                        _ => {}
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
        Self {
            base,
            admin,
            acks,
            heartbeats,
            task,
        }
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let response = reqwest::Client::new()
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.admin)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
    }

    async fn token(&self) -> String {
        serde_json::from_value::<CreateEnrollmentTokenResponse>(
            self.post(
                "/api/v1/enrollment-tokens",
                json!({"expires_in_seconds":3600}),
            )
            .await,
        )
        .unwrap()
        .token
    }
}

// Current-thread runtimes keep the cfg(test), thread-local filesystem fault
// attached to the caller executing the synchronous production write primitive.
#[tokio::test]
async fn post_commit_cache_failure_blocks_ack_through_retry_and_restart_until_durable() {
    use std::os::unix::fs::MetadataExt;
    let directory = directory();
    let server = Server::start(&directory.path().join("controller")).await;
    let path = directory.path().join("agent.json");
    let cache_path = directory.path().join("agent.json.config.json");
    let token = server.token().await;
    let mut agent = Agent::prepare_enrollment(&path, &server.base, &token, "durability").unwrap();
    let node = agent.enroll(&token).await.unwrap().node;
    agent.sync().await.unwrap();
    assert_eq!(server.acks.load(Ordering::SeqCst), 1);
    server
        .post(
            "/api/v1/nodes/country",
            json!({"node_id":node.id,"country":"US","expected_revision":2}),
        )
        .await;

    let fault = DirectorySyncFailure::after_publication(&cache_path);
    assert!(matches!(
        agent.sync().await,
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let visible: Value = serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    assert_eq!(visible["snapshot"]["payload"]["revision"], 3);
    assert_eq!(
        agent.status().stored_revision,
        3,
        "memory must retain the published replay floor"
    );
    assert_eq!(server.acks.load(Ordering::SeqCst), 1);
    let inode = std::fs::metadata(&cache_path).unwrap().ino();

    // The next payload is identical: skipping the write must not skip fsync.
    assert!(matches!(agent.sync().await, Err(AgentError::File(_))));
    assert_eq!(server.acks.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::metadata(&cache_path).unwrap().ino(), inode);
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert_eq!(agent.status().stored_revision, 3);
    assert!(matches!(agent.sync().await, Err(AgentError::File(_))));
    assert_eq!(server.acks.load(Ordering::SeqCst), 1);
    drop(fault);
    let receipt = agent.sync().await.unwrap();
    assert_eq!(receipt.stored_revision, 3);
    assert_eq!(server.acks.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::metadata(&cache_path).unwrap().ino(),
        inode,
        "durability recovery does not rewrite unchanged cache"
    );
}

#[tokio::test]
async fn post_commit_identity_failure_preserves_visible_binding_and_requires_sync_before_use() {
    let directory = directory();
    let server = Server::start(&directory.path().join("controller")).await;
    let path = directory.path().join("agent.json");
    let token = server.token().await;
    let mut agent = Agent::prepare_enrollment(&path, &server.base, &token, "identity").unwrap();
    let fault = DirectorySyncFailure::after_publication(&path);
    assert!(matches!(
        agent.enroll(&token).await,
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let visible = Identity::load(&path).unwrap();
    assert!(visible.node_id.is_some());
    assert_eq!(
        agent.status().node_id,
        visible.node_id,
        "do not roll back only the in-memory identity"
    );
    assert!(matches!(agent.heartbeat().await, Err(AgentError::File(_))));
    assert_eq!(server.heartbeats.load(Ordering::SeqCst), 0);
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert!(matches!(agent.heartbeat().await, Err(AgentError::File(_))));
    assert_eq!(server.heartbeats.load(Ordering::SeqCst), 0);
    drop(fault);
    assert_eq!(
        Some(agent.heartbeat().await.unwrap().node.id),
        visible.node_id
    );
    assert_eq!(server.heartbeats.load(Ordering::SeqCst), 1);
}

#[test]
fn post_commit_identity_creation_returns_error_without_discarding_or_replacing_credential() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let token = local::credential().unwrap();
    let fault = DirectorySyncFailure::after_publication(&path);
    assert!(matches!(
        Agent::prepare_enrollment(&path, "http://127.0.0.1:1", &token, "pending"),
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let before = std::fs::read(&path).unwrap();
    drop(fault);
    let agent = Agent::prepare_enrollment(&path, "http://127.0.0.1:1", &token, "pending").unwrap();
    assert_eq!(agent.status().enrollment, "pending");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
