use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{Router, response::IntoResponse};
use meshway_controller::{AppState, admin_token, router};
use meshway_protocol::{
    AgentConnectionsSnapshot, CreateEnrollmentTokenResponse, connection_digest,
};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    super::connection_key, Agent, AgentError, cache_path, connections_status, convert, local,
};
use meshway_control_client::client::HttpClient;

use crate::test_network_scope as network_scope;

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[derive(Default)]
struct Observed {
    publications: AtomicUsize,
    pulls: AtomicUsize,
    acks: AtomicUsize,
    mode: AtomicUsize,
}

struct Server {
    base: String,
    admin: String,
    observed: Arc<Observed>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(directory: &Path, state_path: PathBuf) -> Self {
        let state = AppState::open(directory).unwrap();
        let admin = admin_token(directory).unwrap();
        let observed = Arc::new(Observed::default());
        let watched = observed.clone();
        let app: Router = router(state).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let observed = watched.clone();
                let state_path = state_path.clone();
                async move {
                    let publication = request.uri().path() == "/api/v1/agent/connection-identity";
                    let pull = request.uri().path() == "/api/v1/agent/connections";
                    let ack = request.uri().path() == "/api/v1/agent/connections/ack";
                    let mode = observed.mode.load(Ordering::SeqCst);
                    let request = if publication {
                        observed.publications.fetch_add(1, Ordering::SeqCst);
                        // Every actual HTTP publication observes the existing private sidecar.
                        let stored: serde_json::Value = serde_json::from_str(
                            &local::read_private(&connection_key::path(&state_path)).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            std::fs::metadata(connection_key::path(&state_path))
                                .unwrap()
                                .mode()
                                & 0o777,
                            0o600
                        );
                        let (parts, body) = request.into_parts();
                        let bytes = axum::body::to_bytes(body, 8192).await.unwrap();
                        let body: meshway_protocol::PublishConnectionIdentityRequest =
                            serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(body.public_key, stored["public_key"]);
                        assert!(
                            !String::from_utf8_lossy(&bytes)
                                .contains(stored["private_key"].as_str().unwrap())
                        );
                        axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes))
                    } else {
                        request
                    };
                    if pull {
                        observed.pulls.fetch_add(1, Ordering::SeqCst);
                        if mode == 401 || mode == 403 {
                            return axum::http::StatusCode::from_u16(mode as u16)
                                .unwrap()
                                .into_response();
                        }
                    }
                    let request = if ack {
                        let attempt = observed.acks.fetch_add(1, Ordering::SeqCst);
                        let stored: super::Cache = serde_json::from_str(
                            &local::read_private_bounded(&cache_path(&state_path), 2 << 20)
                                .unwrap(),
                        )
                        .unwrap();
                        let (parts, body) = request.into_parts();
                        let bytes = axum::body::to_bytes(body, 8192).await.unwrap();
                        let ack: meshway_protocol::AcknowledgeAgentConnectionsRequest =
                            serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(ack.digest, stored.snapshot.digest);
                        assert_eq!(ack.revision, stored.snapshot.payload.revision);
                        if mode == 409 || (mode == 410 && attempt < 2) {
                            return axum::http::StatusCode::CONFLICT.into_response();
                        }
                        axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes))
                    } else {
                        request
                    };
                    let response = next.run(request).await;
                    if publication && mode == 503 {
                        assert_eq!(response.status(), 200);
                        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    if !pull || !(1..=10).contains(&mode) {
                        return response;
                    }
                    let (mut parts, body) = response.into_parts();
                    let bytes = axum::body::to_bytes(body, 2 << 20).await.unwrap();
                    let mut snapshot: AgentConnectionsSnapshot =
                        serde_json::from_slice(&bytes).unwrap();
                    match mode {
                        1 => snapshot.digest = "0".repeat(64),
                        2 => snapshot.payload.revision -= 1,
                        3 => {
                            snapshot.payload.valid = false;
                            snapshot.payload.diagnostics.push(
                                meshway_protocol::ConnectionDiagnostic {
                                    item_id: None,
                                    field: "configuration".into(),
                                    message: "Invalid source".into(),
                                },
                            );
                        }
                        4 => snapshot.payload.node_id = "4".repeat(64),
                        5 => snapshot.payload.public_key = None,
                        6 => snapshot
                            .payload
                            .peers
                            .push(meshway_protocol::ConnectionPeer {
                                node_id: "4".repeat(64),
                                public_key: snapshot.payload.public_key.clone().unwrap(),
                            }),
                        7 => snapshot.payload.incoming.push(
                            meshway_protocol::IncomingConnectionApproval {
                                candidate_id: "unsafe".into(),
                                listener_id: "missing".into(),
                                peer_node_id: "4".repeat(64),
                                transport: meshway_protocol::ConnectionTransport::NativeTcp,
                            },
                        ),
                        8 => snapshot.payload.capabilities.push(
                            meshway_protocol::ConnectionCapability {
                                transport: meshway_protocol::ConnectionTransport::NativeUdp,
                                can_dial: true,
                                can_accept: true,
                                semantics: meshway_protocol::ConnectionSemantics::Stream,
                            },
                        ),
                        9 => snapshot.payload.schema_version = 2,
                        10 => {}
                        _ => unreachable!(),
                    }
                    if (4..=9).contains(&mode) {
                        snapshot.payload.revision += 1;
                    }
                    if mode != 1 {
                        snapshot.digest = connection_digest(&snapshot.payload).unwrap();
                    }
                    let bytes = if mode == 10 {
                        let mut value = serde_json::to_value(snapshot).unwrap();
                        value["payload"]["private_key"] = "unexpected credential".into();
                        serde_json::to_vec(&value).unwrap()
                    } else {
                        serde_json::to_vec(&snapshot).unwrap()
                    };
                    parts.headers.remove("content-length");
                    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
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
            observed,
            task,
        }
    }

    // Test-only admin fixture setup; production agent networking uses only SDK operations.
    async fn admin_post<T: DeserializeOwned>(&self, path: &str, body: impl Serialize) -> T {
        let response = reqwest::Client::new()
            .post(network_scope::admin_url(&self.base, &self.admin, path).await)
            .bearer_auth(&self.admin)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
    }

    async fn agent(&self, path: &Path) -> Agent {
        let token: CreateEnrollmentTokenResponse = self
            .admin_post(
                "/enrollment-tokens",
                meshway_protocol::CreateEnrollmentTokenRequest {
                    expires_in_seconds: 3600,
                },
            )
            .await;
        let mut agent =
            Agent::prepare_enrollment(path, &self.base, &token.token, "connections").unwrap();
        agent.enroll(&token.token).await.unwrap();
        agent
    }
}

#[tokio::test]
async fn not_committed_initial_key_write_never_publishes_or_acks_and_retry_is_stable() {
    let directory = directory();
    let path = directory.path().join("agent/agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    let identity_before = std::fs::read(&path).unwrap();
    let key_path = connection_key::path(&path);
    let fault = local::precommit_faults::ParentUnavailable::during_write(&key_path);

    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::Write(local::AtomicWriteError::NotCommitted(error)))
            if error.kind() == std::io::ErrorKind::NotADirectory
    ));
    assert!(!key_path.exists());
    assert!(!cache_path(&path).exists());
    let status = agent.connection_status().unwrap();
    assert!(status.public_key.is_none());
    assert_eq!(status.stored_revision, 0);
    assert_eq!(server.observed.publications.load(Ordering::SeqCst), 0);
    assert_eq!(server.observed.pulls.load(Ordering::SeqCst), 0);
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);
    assert_eq!(
        std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700,
        "the real private parent must be restored even while the fault is armed"
    );

    drop(fault);
    let receipt = agent.sync_connections().await.unwrap();
    assert_eq!(server.observed.publications.load(Ordering::SeqCst), 1);
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    let key_bytes = std::fs::read(&key_path).unwrap();
    let key_inode = std::fs::metadata(&key_path).unwrap().ino();
    let public_key = agent.connection_status().unwrap().public_key.unwrap();
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert_eq!(
        agent.sync_connections().await.unwrap().digest,
        receipt.digest
    );
    assert_eq!(
        agent.connection_status().unwrap().public_key.unwrap(),
        public_key
    );
    assert_eq!(std::fs::read(&key_path).unwrap(), key_bytes);
    assert_eq!(std::fs::metadata(&key_path).unwrap().ino(), key_inode);
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);
}

#[tokio::test]
async fn not_committed_cache_replacement_preserves_cache_replay_floor_and_published_key() {
    let directory = directory();
    let path = directory.path().join("agent/agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    let receipt = agent.sync_connections().await.unwrap();
    let cached_path = cache_path(&path);
    let cache_before = std::fs::read(&cached_path).unwrap();
    let cache_inode = std::fs::metadata(&cached_path).unwrap().ino();
    let key_path = connection_key::path(&path);
    let key_before = std::fs::read(&key_path).unwrap();
    let key_inode = std::fs::metadata(&key_path).unwrap().ino();
    let identity_before = std::fs::read(&path).unwrap();
    let _: meshway_protocol::RevokeNodeResponse = server
        .admin_post(
            "/nodes/country",
            meshway_protocol::SetNodeCountryRequest {
                node_id: receipt.node_id.clone(),
                country: Some("US".into()),
                expected_revision: receipt.stored_revision,
            },
        )
        .await;
    let fault = local::precommit_faults::ParentUnavailable::during_write(&cached_path);

    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::Write(local::AtomicWriteError::NotCommitted(error)))
            if error.kind() == std::io::ErrorKind::NotADirectory
    ));
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    let status = agent.connection_status().unwrap();
    assert_eq!(status.stored_revision, receipt.stored_revision);
    assert_eq!(
        status.stored_digest.as_deref(),
        Some(receipt.digest.as_str())
    );
    assert_eq!(std::fs::read(&cached_path).unwrap(), cache_before);
    assert_eq!(std::fs::metadata(&cached_path).unwrap().ino(), cache_inode);
    assert_eq!(std::fs::read(&key_path).unwrap(), key_before);
    assert_eq!(std::fs::metadata(&key_path).unwrap().ino(), key_inode);
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);

    // The visible cache and in-memory replay floor remain the previous snapshot.
    let mut rollback = agent
        .connections
        .as_ref()
        .unwrap()
        .cache
        .as_ref()
        .unwrap()
        .snapshot
        .clone();
    rollback.payload.revision -= 1;
    rollback.digest = connection_digest(&rollback.payload).unwrap();
    assert!(matches!(
        agent
            .connections
            .as_mut()
            .unwrap()
            .persist(&path, &agent.identity, rollback),
        Err(AgentError::Invalid(_))
    ));
    assert_eq!(
        agent.connection_status().unwrap().stored_revision,
        receipt.stored_revision
    );
    assert_eq!(std::fs::read(&cached_path).unwrap(), cache_before);
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);

    drop(fault);
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert_eq!(
        agent.connection_status().unwrap().stored_revision,
        receipt.stored_revision
    );
    let retried = agent.sync_connections().await.unwrap();
    assert_eq!(retried.stored_revision, receipt.stored_revision + 1);
    assert_ne!(retried.digest, receipt.digest);
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&key_path).unwrap(), key_before);
    assert_eq!(std::fs::metadata(&key_path).unwrap().ino(), key_inode);
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);
}

#[tokio::test]
async fn key_fsync_failure_prevents_every_publication_including_restart() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    let identity_before = std::fs::read(&path).unwrap();
    let key_path = connection_key::path(&path);
    let fault = local::test_faults::DirectorySyncFailure::after_publication(&key_path);
    assert!(matches!(
        agent.publish_connection_identity().await,
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let before = std::fs::read(&key_path).unwrap();
    assert!(matches!(
        agent.publish_connection_identity().await,
        Err(AgentError::File(_))
    ));
    assert_eq!(server.observed.publications.load(Ordering::SeqCst), 0);
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert!(matches!(
        agent.publish_connection_identity().await,
        Err(AgentError::File(_))
    ));
    assert_eq!(server.observed.publications.load(Ordering::SeqCst), 0);
    drop(fault);
    let published = agent.publish_connection_identity().await.unwrap();
    assert_eq!(server.observed.publications.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(&key_path).unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);
    assert_eq!(
        agent.connection_status().unwrap().public_key.unwrap(),
        published.public_key
    );
}

#[tokio::test]
async fn lost_publication_response_reuses_key_and_keeps_identity_bytes_and_secrets_private() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    let identity_before = std::fs::read(&path).unwrap();
    server.observed.mode.store(503, Ordering::SeqCst);
    assert!(matches!(
        agent.publish_connection_identity().await,
        Err(AgentError::Http {
            status: 503,
            retryable: true
        })
    ));
    let key_before = std::fs::read(connection_key::path(&path)).unwrap();
    drop(agent);
    server.observed.mode.store(0, Ordering::SeqCst);
    let mut agent = Agent::load(&path).unwrap();
    let publication = agent.publish_connection_identity().await.unwrap();
    assert_eq!(publication.revision, 3);
    assert_eq!(
        agent.publish_connection_identity().await.unwrap().revision,
        3
    );
    assert_eq!(
        std::fs::read(connection_key::path(&path)).unwrap(),
        key_before
    );
    assert_eq!(std::fs::read(&path).unwrap(), identity_before);
    let secret: serde_json::Value = serde_json::from_slice(&key_before).unwrap();
    let status = agent.connection_status().unwrap();
    let public = serde_json::to_string(&status).unwrap();
    assert!(!public.contains(secret["private_key"].as_str().unwrap()));
    assert!(!format!("{status:?}").contains(secret["private_key"].as_str().unwrap()));
    assert!(!public.contains(&agent.identity.node_credential));
    assert!(!status.applied);
}

#[tokio::test]
async fn committed_cache_fsync_failure_preserves_replay_floor_and_never_acks_until_durable() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    let fault = local::test_faults::DirectorySyncFailure::after_publication(&cache_path(&path));
    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let before = std::fs::read(cache_path(&path)).unwrap();
    let inode = std::fs::metadata(cache_path(&path)).unwrap().ino();
    assert_eq!(agent.connection_status().unwrap().stored_revision, 3);
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 0);
    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::File(_))
    ));
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 0);
    drop(agent);
    let mut agent = Agent::load(&path).unwrap();
    assert_eq!(agent.connection_status().unwrap().stored_revision, 3);
    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::File(_))
    ));
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 0);
    drop(fault);
    let receipt = agent.sync_connections().await.unwrap();
    assert_eq!(receipt.stored_revision, 3);
    assert!(!receipt.applied);
    assert_eq!(
        receipt.status,
        meshway_protocol::ConnectionReceiptStatus::Saved
    );
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(cache_path(&path)).unwrap(), before);
    assert_eq!(std::fs::metadata(cache_path(&path)).unwrap().ino(), inode);
    assert_eq!(
        std::fs::metadata(cache_path(&path)).unwrap().mode() & 0o777,
        0o600
    );
    assert!(!directory.path().join("agent.json.config.json").exists());
    // An unsafe parent is rejected before the next network publication or ACK.
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(agent.sync_connections().await.is_err());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bad_hash_replay_mutation_binding_and_scope_fail_before_disk_write_or_ack() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    agent.sync_connections().await.unwrap();
    let before = std::fs::read(cache_path(&path)).unwrap();
    for mode in 1..=9 {
        server.observed.mode.store(mode, Ordering::SeqCst);
        assert!(
            agent.sync_connections().await.is_err(),
            "invalid response {mode}"
        );
        assert_eq!(std::fs::read(cache_path(&path)).unwrap(), before);
        assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    }
    server.observed.mode.store(0, Ordering::SeqCst);
    for field in ["controller", "node_id"] {
        let mut changed: serde_json::Value = serde_json::from_slice(&before).unwrap();
        changed[field] = "different".into();
        let changed = serde_json::to_vec(&changed).unwrap();
        local::atomic_write(&cache_path(&path), &changed, true).unwrap();
        assert!(connections_status(&path).is_err());
        assert!(agent.sync_connections().await.is_err());
        assert_eq!(std::fs::read(cache_path(&path)).unwrap(), changed);
        assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    }
    local::atomic_write(&cache_path(&path), &before, true).unwrap();
    std::fs::set_permissions(cache_path(&path), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(agent.sync_connections().await.is_err());
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 1);
    std::fs::set_permissions(cache_path(&path), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(connection_key::path(&path)).unwrap();
    assert!(connections_status(&path).is_err());
    assert!(agent.sync_connections().await.is_err());
    drop(agent);
    assert!(
        Agent::load(&path)
            .unwrap()
            .sync_connections()
            .await
            .is_err()
    );
    assert!(!connection_key::path(&path).exists());
}

#[tokio::test]
async fn unknown_response_fields_fail_before_persistence_or_ack() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    server.observed.mode.store(10, Ordering::SeqCst);
    assert!(agent.sync_connections().await.is_err());
    assert!(
        !cache_path(&path).exists(),
        "closed schemas must reject unexpected credentials before persisting"
    );
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ack_races_repull_three_times_but_auth_failures_are_fatal() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    server.observed.mode.store(410, Ordering::SeqCst);
    agent.sync_connections().await.unwrap();
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 3);
    assert_eq!(server.observed.pulls.load(Ordering::SeqCst), 3);
    server.observed.mode.store(409, Ordering::SeqCst);
    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::Http {
            status: 409,
            retryable: true
        })
    ));
    assert_eq!(server.observed.acks.load(Ordering::SeqCst), 6);
    assert_eq!(server.observed.pulls.load(Ordering::SeqCst), 6);
    for status in [401, 403] {
        server.observed.mode.store(status, Ordering::SeqCst);
        assert!(
            matches!(agent.sync_connections().await, Err(AgentError::Http { status: code, retryable: false }) if usize::from(code) == status)
        );
        assert_eq!(server.observed.acks.load(Ordering::SeqCst), 6);
    }
}

#[tokio::test]
async fn immutable_binding_conflict_and_revocation_preserve_key_and_identity() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let server = Server::start(&directory.path().join("controller"), path.clone()).await;
    let mut agent = server.agent(&path).await;
    agent.publish_connection_identity().await.unwrap();
    let identity = std::fs::read(&path).unwrap();
    let key = std::fs::read(connection_key::path(&path)).unwrap();
    // A valid but externally substituted sidecar is never an implicit rotation.
    let other_path = directory.path().join("other.json");
    local::atomic_write(&other_path, &identity, false).unwrap();
    let mut other_keys = super::KeyStore::load(&other_path, &agent.identity).unwrap();
    other_keys.prepare(&other_path, &agent.identity).unwrap();
    let other = std::fs::read(connection_key::path(&other_path)).unwrap();
    drop(agent);
    local::atomic_write(&connection_key::path(&path), &other, true).unwrap();
    let mut agent = Agent::load(&path).unwrap();
    assert!(matches!(
        agent.publish_connection_identity().await,
        Err(AgentError::Http {
            status: 409,
            retryable: false
        })
    ));
    assert_eq!(std::fs::read(connection_key::path(&path)).unwrap(), other);
    assert_eq!(std::fs::read(&path).unwrap(), identity);
    drop(agent);
    local::atomic_write(&connection_key::path(&path), &key, true).unwrap();
    let mut agent = Agent::load(&path).unwrap();
    // Real role mismatch decoding through the generated node-only SDK operation.
    let error = HttpClient::new()
        .with_base_url(&server.base)
        .with_api_key(&server.admin)
        .publish_connection_identity(
            convert(meshway_protocol::PublishConnectionIdentityRequest {
                public_key: agent.connection_status().unwrap().public_key.unwrap(),
                capabilities: super::compiled_connection_capabilities(),
            })
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        AgentError::from(error),
        AgentError::Http {
            status: 401,
            retryable: false
        }
    ));
    let _: meshway_protocol::RevokeNodeResponse = server
        .admin_post(
            "/nodes/revoke",
            meshway_protocol::RevokeNodeRequest {
                node_id: agent.identity.node_id.clone().unwrap(),
            },
        )
        .await;
    assert!(matches!(
        agent.sync_connections().await,
        Err(AgentError::Http {
            status: 403,
            retryable: false
        })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), identity);
    assert_eq!(std::fs::read(connection_key::path(&path)).unwrap(), key);
}
