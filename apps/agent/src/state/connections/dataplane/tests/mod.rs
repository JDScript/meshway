use super::*;
use crate::managed_dataplane::{
    PacketPlan, TunOptions,
    tests::{MockDevice, fixtures, snapshot},
    tun::Tun,
};
use crate::state::connection_key;
use meshway_protocol::{
    AcknowledgeAgentConnectionsRequest, AcknowledgeConfigurationRequest,
    AcknowledgeConfigurationResponse, ConnectionRuntimeReason, ConnectionRuntimeState,
    NodeMeshExecution, PublishConnectionIdentityResponse,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod dns;
mod revocation;
mod stale;

static RUNTIME: std::sync::LazyLock<MeshExecutionRequest> =
    std::sync::LazyLock::new(|| MeshExecutionRequest {
        epoch: 1,
        instance_id: "a".repeat(64),
        capabilities: vec![MeshExecutionCapability::Ipv4StatefulExactPathV2],
    });

fn managed(snapshot: MeshExecutionSnapshot) -> MeshExecutionResponse {
    MeshExecutionResponse {
        revision: snapshot.payload.revision,
        snapshot,
    }
}

fn projections(
    snapshot: &MeshExecutionSnapshot,
) -> (ConfigurationSnapshot, AgentConnectionsSnapshot) {
    let payload = NodeConfiguration {
        schema_version: CONFIGURATION_SCHEMA_VERSION,
        revision: snapshot.payload.revision,
        network: snapshot.payload.network.clone(),
        node: snapshot.payload.node.clone(),
        peers: snapshot.payload.peers.clone(),
    };
    (
        ConfigurationSnapshot {
            digest: configuration_digest(&payload).unwrap(),
            payload,
        },
        AgentConnectionsSnapshot {
            digest: connection_digest(&snapshot.payload.connections).unwrap(),
            payload: snapshot.payload.connections.clone(),
        },
    )
}

async fn execution_ack(
    axum::Json(request): axum::Json<AcknowledgeMeshExecutionRequest>,
) -> axum::Json<AcknowledgeMeshExecutionResponse> {
    assert_eq!(request.runtime.epoch, 1);
    assert_eq!(request.runtime.capabilities, RUNTIME.capabilities);
    axum::Json(AcknowledgeMeshExecutionResponse {
        revision: request.revision,
        digest: request.digest,
        saved: true,
    })
}

#[test]
fn coherent_validation_rejects_digest_binding_revision_and_identity_scope_mismatches() {
    let dir = directory();
    let (agent, local) = enrolled(dir.path(), "http://127.0.0.1");
    let (mut payloads, _) = fixtures(3);
    payloads[0].connections.public_key = local.connections.public_key;
    let original = snapshot(payloads[0].clone());
    let own_key = original.payload.connections.public_key.as_deref().unwrap();
    validate(&agent, &original, None).unwrap();
    for mode in 0..11 {
        let mut value = original.clone();
        match mode {
            0 => value.payload.schema_version = MESH_EXECUTION_VERSION + 1,
            1 => value.payload.revision += 1,
            2 => value.payload.node_id = "f".repeat(64),
            3 => value.payload.identities.pop().map(|_| ()).unwrap(),
            4 => value.payload.identities[0].public_key = own_key.into(),
            5 => {
                value.payload.identities[0].public_key = meshway_dataplane::Identity::generate()
                    .unwrap()
                    .public_key_hex()
            }
            6 => {
                value.payload.paths[0].nodes.remove(0);
            }
            7 => value.payload.valid = false,
            8 => value.payload.node.mesh_ipv4 = "100.64.0.20".into(),
            9 => {
                value.payload.connections.public_key = Some(
                    meshway_dataplane::Identity::generate()
                        .unwrap()
                        .public_key_hex(),
                )
            }
            10 => value.payload.identities.reverse(),
            _ => unreachable!(),
        }
        value.digest = mesh_execution_digest(&value.payload).unwrap();
        assert!(validate(&agent, &value, None).is_err(), "mode {mode}");
    }
    let mut value = original.clone();
    value.digest = "f".repeat(64);
    assert!(validate(&agent, &value, None).is_err());
    let mut rollback = original.clone();
    rollback.payload.revision -= 1;
    rollback.payload.connections.revision -= 1;
    rollback.digest = mesh_execution_digest(&rollback.payload).unwrap();
    assert!(validate(&agent, &rollback, Some(&original)).is_err());
    let mut mutation = original.clone();
    mutation.payload.paths.clear();
    mutation.payload.identities = mutation.payload.connections.peers.clone();
    mutation.digest = mesh_execution_digest(&mutation.payload).unwrap();
    assert!(validate(&agent, &mutation, Some(&original)).is_err());
}

fn enrolled(directory: &Path, controller: &str) -> (Agent, NodeMeshExecution) {
    let (payloads, _) = fixtures(2);
    let mut payload = payloads[0].clone();
    let mut agent = Agent::prepare_enrollment(
        directory.join("agent.json"),
        controller,
        &"a".repeat(64),
        &payload.node.name,
    )
    .unwrap();
    agent.identity.node_id = Some(payload.node_id.clone());
    crate::state::write_identity(&agent.path, &agent.identity, true).unwrap();
    agent.load_connections().unwrap();
    let key = agent
        .connections
        .as_mut()
        .unwrap()
        .keys
        .prepare(&agent.path, &agent.identity)
        .unwrap();
    payload.connections.public_key = Some(key.into());
    (agent, payload)
}

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[test]
fn dataplane_cache_is_private_bound_to_credential_and_reuses_existing_identity_key() {
    let dir = directory();
    let (agent, payload) = enrolled(dir.path(), "http://127.0.0.1");
    let key_file = connection_key::path(&agent.path);
    let key = local::read_private(&key_file).unwrap();
    let public = payload.connections.public_key.clone();
    let state_path = agent.path.clone();
    let mut state = State { cache: None };
    state.persist(&agent, snapshot(payload)).unwrap();
    assert_eq!(
        std::fs::metadata(path(&agent))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(agent);
    let mut agent = Agent::load(&state_path).unwrap();
    let cache = State::load(&mut agent).unwrap();
    assert!(cache.has_cache());
    assert_eq!(
        agent.connections.as_ref().unwrap().keys.public_key(),
        public.as_deref()
    );
    assert_eq!(local::read_private(&key_file).unwrap(), key);
    agent.identity.node_credential = "b".repeat(64);
    assert!(load(&agent).is_err());
}

#[test]
fn unsupported_packet_cache_is_refused_without_rebinding_or_removal() {
    let dir = directory();
    let (mut agent, _) = enrolled(dir.path(), "http://127.0.0.1");
    let identity = local::read_private(&agent.path).unwrap();
    let key_path = connection_key::path(&agent.path);
    let key = local::read_private(&key_path).unwrap();
    let old_path = agent.path.with_extension("json.dataplane.json");
    local::atomic_write(&old_path, b"{}", false).unwrap();
    assert!(matches!(
        State::load(&mut agent),
        Err(AgentError::Invalid(
            "unsupported old dataplane cache; explicit cleanup required"
        ))
    ));
    assert_eq!(local::read_private(&old_path).unwrap(), "{}");
    assert_eq!(local::read_private(&agent.path).unwrap(), identity);
    assert_eq!(local::read_private(&key_path).unwrap(), key);
}

#[tokio::test]
async fn changed_policy_withdraws_before_stalled_ack() {
    let dir = directory();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (mut agent, payload) = enrolled(dir.path(), &format!("http://{address}"));
    let current = Arc::new(Mutex::new(snapshot(payload.clone())));
    let stalled = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(tokio::sync::Notify::new());
    // The policy change races after publication. Keep publication at R so this
    // test specifically covers invalidation on the subsequently fetched R+1.
    let published_revision = payload.revision;
    let app = axum::Router::new()
        .route(
            "/api/v1/agent/connection-identity",
            axum::routing::post({
                let current = current.clone();
                move || {
                    let current = current.clone();
                    async move {
                        let value = current.lock().unwrap();
                        axum::Json(PublishConnectionIdentityResponse {
                            revision: published_revision,
                            node_id: value.payload.node_id.clone(),
                            public_key: value.payload.connections.public_key.clone().unwrap(),
                            capabilities: value.payload.connections.capabilities.clone(),
                        })
                    }
                }
            }),
        )
        .route(
            "/api/v1/agents/mesh-execution",
            axum::routing::post({
                let current = current.clone();
                move || {
                    let value = current.lock().unwrap().clone();
                    async move { axum::Json(managed(value)) }
                }
            }),
        )
        .route(
            "/api/v1/agents/configuration",
            axum::routing::get({
                let current = current.clone();
                move || {
                    let inventory = projections(&current.lock().unwrap()).0;
                    async move { axum::Json(inventory) }
                }
            }),
        )
        .route(
            "/api/v1/agents/mesh-execution/ack",
            axum::routing::post(execution_ack),
        )
        .route(
            "/api/v1/agents/configuration/ack",
            axum::routing::post({
                let stalled = stalled.clone();
                let entered = entered.clone();
                move |axum::Json(request): axum::Json<AcknowledgeConfigurationRequest>| {
                    let stalled = stalled.clone();
                    let entered = entered.clone();
                    async move {
                        if stalled.load(Ordering::SeqCst) {
                            entered.notify_one();
                            std::future::pending::<()>().await;
                        }
                        axum::Json(AcknowledgeConfigurationResponse {
                            revision: request.revision,
                            digest: request.digest,
                        })
                    }
                }
            }),
        )
        .route(
            "/api/v1/agent/connections/ack",
            axum::routing::post(
                |axum::Json(request): axum::Json<AcknowledgeAgentConnectionsRequest>| async move {
                    axum::Json(AcknowledgeAgentConnectionsResponse {
                        revision: request.revision,
                        digest: request.digest,
                        status: ConnectionReceiptStatus::Saved,
                    })
                },
            ),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut cache = State::load(&mut agent).unwrap();
    let (device, _send, _receive) = MockDevice::new();
    let mut manager = Manager::new_dataplane(
        None,
        Duration::from_secs(30),
        TunOptions {
            name: "mock0".into(),
            mtu: 1000,
        },
        agent.identity.controller.clone(),
        false,
    );
    manager.mock_tun(Tun::mock(
        device,
        &PacketPlan::compile(&payload, 1000).unwrap(),
    ));
    let fresh = cache
        .sync(&mut agent, &mut manager, &RUNTIME)
        .await
        .unwrap();
    let key = agent
        .connections
        .as_ref()
        .unwrap()
        .keys
        .application_key(&agent.path, &agent.identity)
        .unwrap();
    manager
        .reconcile_dataplane(fresh.snapshot, key, fresh.fresh_at)
        .await
        .unwrap();
    let (_, proof) = manager.report_snapshot();
    let proof = proof.unwrap();
    assert!(proof.current());
    {
        let mut value = current.lock().unwrap();
        value.payload.revision += 1;
        value.payload.connections.revision += 1;
        value.payload.paths.clear();
        value.digest = mesh_execution_digest(&value.payload).unwrap();
    }
    stalled.store(true, Ordering::SeqCst);
    {
        let sync = cache.sync(&mut agent, &mut manager, &RUNTIME);
        tokio::pin!(sync);
        tokio::select! {
            result = &mut sync => panic!("ACK unexpectedly completed: {}", result.is_ok()),
            result = tokio::time::timeout(Duration::from_secs(3), entered.notified()) => { result.unwrap(); },
        }
        assert!(
            !proof.current(),
            "new validated routing intent must already fence old packet generation while ACK stalls"
        );
    }
    assert_eq!(
        manager.observation().state,
        ConnectionRuntimeState::Withdrawn
    );
    assert_eq!(
        cache.cache.as_ref().unwrap().snapshot.payload.revision,
        payload.revision + 1
    );
    manager
        .withdraw(ConnectionRuntimeReason::ShuttingDown)
        .await;
    server.abort();
}
