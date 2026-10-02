use meshway_policy as policy;
use meshway_policy::NetworkPolicy;
use meshway_protocol::*;

#[test]
fn policy_default_jsonc_cas_budget_and_saved_receipts_are_current_only() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let scope = store.network_id.clone();
    let initial = store.network_policy(&scope).unwrap();
    assert_eq!(initial.policy, NetworkPolicy::default_template());
    assert!(initial.valid);
    let mut current = initial.policy.clone();
    current.grants.clear();
    let text = format!(
        "// exact draft retained\n{}",
        serde_json::to_string(&current).unwrap()
    );
    let request = SaveNetworkPolicyRequest {
        expected_revision: initial.revision,
        policy: current.clone(),
        source_jsonc: Some(text.clone()),
    };
    let saved = store.save_network_policy(&scope, &request).unwrap();
    assert_eq!(saved.revision, initial.revision + 1);
    assert_eq!(saved.source_jsonc, Some(text));
    assert!(matches!(
        store.save_network_policy(&scope, &request),
        Err(Error::Conflict)
    ));
    let request = SaveNetworkPolicyRequest {
        expected_revision: saved.revision,
        ..request
    };
    assert_eq!(
        store
            .save_network_policy(&scope, &request)
            .unwrap()
            .revision,
        saved.revision
    );
    for bad in [
        Some("{}".into()),
        Some("x".repeat(MAX_SOURCE_JSONC_BYTES + 1)),
    ] {
        assert!(
            store
                .save_network_policy(
                    &scope,
                    &SaveNetworkPolicyRequest {
                        expected_revision: saved.revision,
                        policy: current.clone(),
                        source_jsonc: bad
                    }
                )
                .is_err()
        );
    }
    let mut external = current.clone();
    external.automatic.fallback = "unsupported".into();
    assert!(
        store
            .save_network_policy(
                &scope,
                &SaveNetworkPolicyRequest {
                    expected_revision: saved.revision,
                    policy: external,
                    source_jsonc: None
                }
            )
            .is_err()
    );
    assert_eq!(store.revision().unwrap(), saved.revision);
    let r = crate::store::test_support::request("receipt");
    let node = enroll(&mut store, &r);
    store
        .publish_connection_identity(
            &r.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: format!("09{}", "00".repeat(31)),
                capabilities: vec![],
            },
        )
        .unwrap();
    let instance_id = local::credential().unwrap();
    let begin = store
        .begin_connection_runtime(
            &r.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: instance_id.clone(),
                expected_epoch: 0,
            },
        )
        .unwrap();
    let runtime = MeshExecutionRequest {
        epoch: begin.epoch,
        instance_id,
        capabilities: vec![MeshExecutionCapability::Ipv4ExactPathV1],
    };
    let execution = store
        .mesh_execution(&r.node_credential, &runtime)
        .unwrap()
        .snapshot;
    assert_eq!(execution.payload.network_id, scope.as_str());
    assert_eq!(execution.payload.node_id, node.id);
    assert_eq!(
        mesh_execution_digest(&execution.payload).unwrap(),
        execution.digest
    );
    let ack = AcknowledgeMeshExecutionRequest {
        runtime: runtime.clone(),
        revision: execution.payload.revision,
        digest: execution.digest.clone(),
    };
    assert!(
        store
            .acknowledge_mesh_execution(&r.node_credential, &ack)
            .unwrap()
            .saved
    );
    let wrong = AcknowledgeMeshExecutionRequest {
        digest: "0".repeat(64),
        ..ack.clone()
    };
    assert!(matches!(
        store.acknowledge_mesh_execution(&r.node_credential, &wrong),
        Err(Error::Conflict)
    ));
    store
        .begin_connection_runtime(
            &r.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: local::credential().unwrap(),
                expected_epoch: begin.epoch,
            },
        )
        .unwrap();
    assert!(matches!(
        store.acknowledge_mesh_execution(&r.node_credential, &ack),
        Err(Error::Conflict)
    ));
    drop(store);
    let store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(store.network_policy(&scope).unwrap().policy, current);
}
use super::*;
use crate::{
    NetworkOptions,
    store::test_support::{SingleNetworkFixture, directory, enroll, request},
};

#[test]
fn mixed_epoch_support_withdraws_whole_mandatory_plan_without_replanning_around_old_agent() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let scope = store.network_id.clone();
    let mut participants = Vec::new();
    for seed in 9..12 {
        let r = request(&format!("mixed-{seed}"));
        let n = enroll(&mut store, &r);
        store
            .publish_connection_identity(
                &r.node_credential,
                &PublishConnectionIdentityRequest {
                    public_key: format!("{seed:02x}{}", "00".repeat(31)),
                    capabilities: vec![ConnectionCapability {
                        transport: ConnectionTransport::NativeUdp,
                        semantics: ConnectionSemantics::Datagram,
                        can_dial: true,
                        can_accept: true,
                    }],
                },
            )
            .unwrap();
        let instance = local::credential().unwrap();
        let begin = store
            .begin_connection_runtime(
                &r.node_credential,
                &BeginConnectionRuntimeRequest {
                    instance_id: instance.clone(),
                    expected_epoch: 0,
                },
            )
            .unwrap();
        participants.push((
            n,
            r.node_credential,
            MeshExecutionRequest {
                epoch: begin.epoch,
                instance_id: instance,
                capabilities: vec![MeshExecutionCapability::Ipv4ExactPathV1],
            },
        ));
    }
    let a = &participants[0].0.id;
    let b = &participants[1].0.id;
    let c = &participants[2].0.id;
    let endpoint = ConnectionEndpoint {
        host: "192.0.2.1".into(),
        port: 41000,
    };
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: store.revision().unwrap(),
            configuration: ConnectionPlan {
                listeners: vec![
                    ConnectionListener {
                        id: "b".into(),
                        node_id: b.clone(),
                        transport: ConnectionTransport::NativeUdp,
                        bind: "0.0.0.0:41000".into(),
                        advertised_endpoints: vec![endpoint.clone()],
                        credential_ref: None,
                    },
                    ConnectionListener {
                        id: "c".into(),
                        node_id: c.clone(),
                        transport: ConnectionTransport::NativeUdp,
                        bind: "0.0.0.0:41000".into(),
                        advertised_endpoints: vec![endpoint.clone()],
                        credential_ref: None,
                    },
                ],
                candidates: vec![
                    ConnectionCandidate {
                        id: "ab".into(),
                        dialer_node_id: a.clone(),
                        listener_id: "b".into(),
                        endpoint: endpoint.clone(),
                        priority: 0,
                        credential_ref: None,
                    },
                    ConnectionCandidate {
                        id: "cb".into(),
                        dialer_node_id: c.clone(),
                        listener_id: "b".into(),
                        endpoint: endpoint.clone(),
                        priority: 0,
                        credential_ref: None,
                    },
                    ConnectionCandidate {
                        id: "ac".into(),
                        dialer_node_id: a.clone(),
                        listener_id: "c".into(),
                        endpoint,
                        priority: 0,
                        credential_ref: None,
                    },
                ],
            },
        })
        .unwrap();
    for index in [0, 2] {
        store
            .mesh_execution(&participants[index].1, &participants[index].2)
            .unwrap();
    }
    let policy = NetworkPolicy {
        grants: vec![policy::Grant {
            id: "one-way".into(),
            src: vec![format!("node:{a}")],
            dst: vec![format!("node:{c}")],
            bidirectional: false,
        }],
        mesh: vec![policy::MeshRule {
            id: "mandatory-b".into(),
            src: "*".into(),
            dst: "*".into(),
            via: vec![format!("node:{b}")],
        }],
        ..NetworkPolicy::default_template()
    };
    let revision = store.revision().unwrap();
    store
        .save_network_policy(
            &scope,
            &SaveNetworkPolicyRequest {
                expected_revision: revision,
                policy,
                source_jsonc: None,
            },
        )
        .unwrap();
    let denied = store
        .mesh_execution(&participants[0].1, &participants[0].2)
        .unwrap()
        .snapshot;
    assert!(!denied.payload.valid);
    assert!(denied.payload.paths.is_empty());
    assert!(
        denied
            .payload
            .connections
            .outgoing
            .iter()
            .any(|edge| edge.peer_node_id == *c),
        "direct physical candidate is present but cannot replace B"
    );
    store
        .mesh_execution(&participants[1].1, &participants[1].2)
        .unwrap();
    let allowed = store
        .mesh_execution(&participants[0].1, &participants[0].2)
        .unwrap()
        .snapshot;
    assert!(allowed.payload.valid);
    assert_eq!(allowed.payload.paths.len(), 1);
    assert_eq!(
        allowed.payload.paths[0].nodes,
        vec![a.clone(), b.clone(), c.clone()]
    );
    let reverse = store
        .mesh_execution(&participants[2].1, &participants[2].2)
        .unwrap()
        .snapshot;
    assert!(
        reverse
            .payload
            .paths
            .iter()
            .all(|p| p.nodes.first() != Some(c)),
        "single-direction grant cannot authorize reverse initiation or return packets"
    );
    let old_b = participants[1].2.clone();
    store
        .begin_connection_runtime(
            &participants[1].1,
            &BeginConnectionRuntimeRequest {
                instance_id: local::credential().unwrap(),
                expected_epoch: old_b.epoch,
            },
        )
        .unwrap();
    let denied = store
        .mesh_execution(&participants[0].1, &participants[0].2)
        .unwrap()
        .snapshot;
    assert!(!denied.payload.valid);
    assert!(denied.payload.paths.is_empty());
    assert!(matches!(
        store.mesh_execution(&participants[1].1, &old_b),
        Err(Error::Conflict)
    ));
}
