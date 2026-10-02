use super::*;
use crate::{
    NetworkOptions,
    store::test_support::{SingleNetworkFixture, directory, enroll, request},
};

#[test]
fn explicit_cas_activation_preserves_legacy_jsonc_and_withdraws_old_execution() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let request = request("preserved");
    let node = enroll(&mut store, &request);
    let scope = store.network_id.clone();
    store
        .publish_connection_identity(
            &request.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: "09".repeat(32),
                capabilities: vec![],
            },
        )
        .unwrap();
    let source = store.policy().unwrap();
    let mut external = NetworkPolicy::default_template();
    external.grants[0].dst = vec!["autogroup:internet".into()];
    assert!(matches!(
        store.save_network_policy(
            &scope,
            &SaveNetworkPolicyRequest {
                expected_revision: source.revision,
                policy: external,
                source_jsonc: None
            }
        ),
        Err(Error::Unprocessable(_))
    ));
    assert_eq!(store.revision().unwrap(), source.revision);
    assert!(
        store
            .network_policy(&scope)
            .unwrap()
            .migration_proposal
            .is_none(),
        "legacy default reject cannot silently become automatic routing"
    );
    assert!(
        store
            .save_network_policy(
                &scope,
                &SaveNetworkPolicyRequest {
                    expected_revision: source.revision - 1,
                    policy: NetworkPolicy::default_template(),
                    source_jsonc: None
                }
            )
            .is_err()
    );
    let text = serde_json::to_string(&NetworkPolicy::default_template()).unwrap();
    let v3 = store
        .save_network_policy(
            &scope,
            &SaveNetworkPolicyRequest {
                expected_revision: source.revision,
                policy: NetworkPolicy::default_template(),
                source_jsonc: Some(format!("// explicit replacement\n{text}")),
            },
        )
        .unwrap();
    assert_eq!(v3.source_mode, PolicySourceMode::MeshV3);
    assert_eq!(v3.revision, source.revision + 1);
    assert_eq!(store.policy().unwrap().policy, source.policy);
    let legacy = store.agent_dataplane(&request.node_credential).unwrap();
    assert!(!legacy.payload.access_valid);
    assert!(legacy.payload.access_policy.grants.is_empty());
    assert!(!legacy.payload.configuration.policy_valid);
    assert_eq!(legacy.payload.configuration.node.id, node.id);
    let lease = store
        .begin_connection_runtime(
            &request.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: "a".repeat(64),
                expected_epoch: 0,
            },
        )
        .unwrap();
    let runtime = MeshExecutionRequest {
        epoch: lease.epoch,
        instance_id: "a".repeat(64),
        capabilities: vec![MeshExecutionCapability::Ipv4ExactPathV1],
    };
    let current = store
        .mesh_execution(&request.node_credential, &runtime)
        .unwrap()
        .snapshot
        .unwrap();
    assert_eq!(current.payload.node.mesh_ipv4, node.mesh_ipv4);
    let ack = AcknowledgeMeshExecutionRequest {
        runtime: runtime.clone(),
        revision: current.payload.revision,
        digest: current.digest.clone(),
    };
    assert!(
        store
            .acknowledge_mesh_execution(&request.node_credential, &ack)
            .unwrap()
            .saved
    );
    let before = store.revision().unwrap();
    store
        .begin_connection_runtime(
            &request.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: "b".repeat(64),
                expected_epoch: lease.epoch,
            },
        )
        .unwrap();
    assert_eq!(store.revision().unwrap(), before + 1);
    assert!(matches!(
        store.mesh_execution(&request.node_credential, &runtime),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store.acknowledge_mesh_execution(&request.node_credential, &ack),
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM mesh_execution_capabilities",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    drop(store);
    let store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(
        store.network_policy(&store.network_id).unwrap().source_mode,
        PolicySourceMode::MeshV3
    );
    assert_eq!(store.policy().unwrap().policy, source.policy);
}

#[test]
fn v12_schema_gate_rolls_back_fragments_and_preserves_v11_rows() {
    struct Fixture {
        connection: Connection,
    }
    let mut store = Fixture {
        connection: crate::store::test_support::fixtures::release::v11(),
    };
    let before: u32 = store
        .connection
        .query_row("SELECT revision FROM configuration", [], |r| r.get(0))
        .unwrap();
    store
        .connection
        .execute_batch("CREATE TABLE mesh_execution_receipts(dummy TEXT)")
        .unwrap();
    assert!(super::super::release_schema::upgrade(&mut store.connection).is_err());
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert!(
        !store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='network_policy_sources')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    store
        .connection
        .execute_batch("DROP TABLE mesh_execution_receipts")
        .unwrap();
    super::super::release_schema::upgrade(&mut store.connection).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT revision FROM configuration", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT mesh_ipv4 FROM nodes WHERE id=?1",
                [crate::store::test_support::fixtures::release::NODE],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "100.96.0.2"
    );
    let scope = NetworkId::lookup(
        &store.connection,
        crate::store::test_support::fixtures::release::NETWORK,
    )
    .unwrap();
    assert!(!active(&store.connection, &scope).unwrap());
    super::super::release_schema::upgrade(&mut store.connection).unwrap();
    store
        .connection
        .execute_batch("ALTER TABLE mesh_execution_receipts ADD COLUMN unexpected TEXT")
        .unwrap();
    assert!(super::super::release_schema::upgrade(&mut store.connection).is_err());
}

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
        grants: vec![v3::Grant {
            id: "one-way".into(),
            src: vec![format!("node:{a}")],
            dst: vec![format!("node:{c}")],
            bidirectional: false,
        }],
        mesh: vec![v3::MeshRule {
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
        .snapshot
        .unwrap();
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
        .snapshot
        .unwrap();
    assert!(allowed.payload.valid);
    assert_eq!(allowed.payload.paths.len(), 1);
    assert_eq!(
        allowed.payload.paths[0].nodes,
        vec![a.clone(), b.clone(), c.clone()]
    );
    let reverse = store
        .mesh_execution(&participants[2].1, &participants[2].2)
        .unwrap()
        .snapshot
        .unwrap();
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
        .snapshot
        .unwrap();
    assert!(!denied.payload.valid);
    assert!(denied.payload.paths.is_empty());
    assert!(matches!(
        store.mesh_execution(&participants[1].1, &old_b),
        Err(Error::Conflict)
    ));
    let old = store.agent_dataplane(&participants[1].1).unwrap();
    assert!(!old.payload.access_valid);
    assert!(old.payload.access_policy.grants.is_empty());
    // Retired source is not an execution dependency. Simulate corruption only
    // in this private fixture: v3/compatibility publication must not decode or
    // reinterpret that legacy policy to authorize the current network.
    store
        .connection
        .execute(
            "UPDATE configuration SET policy='retired malformed source' WHERE network_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE mesh_access_source SET policy='retired malformed source' WHERE network_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    let current = store
        .mesh_execution(&participants[0].1, &participants[0].2)
        .unwrap();
    assert_eq!(current.source_mode, PolicySourceMode::MeshV3);
    let legacy = store.agent_dataplane(&participants[0].1).unwrap();
    assert!(!legacy.payload.access_valid);
    assert!(!legacy.payload.configuration.policy_valid);
}
