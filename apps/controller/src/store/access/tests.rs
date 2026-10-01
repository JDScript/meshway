use super::*;
use crate::NetworkOptions;
use rusqlite::params;

fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn enrolled(store: &mut Store, seed: u8, publish: bool) -> (String, String) {
    let token = store.create_token(3600).unwrap();
    let credential = local::credential().unwrap();
    let node = store
        .enroll(
            &token.token,
            &EnrollAgentRequest {
                node_credential: credential.clone(),
                name: format!("node-{seed}"),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    if publish {
        bind(store, &credential, seed);
    }
    (node.id, credential)
}

fn bind(store: &mut Store, token: &str, seed: u8) {
    store
        .publish_connection_identity(
            token,
            &PublishConnectionIdentityRequest {
                public_key: format!("{seed:02x}{}", "00".repeat(31)),
                capabilities: vec![ConnectionCapability {
                    transport: ConnectionTransport::NativeTcp,
                    can_dial: true,
                    can_accept: true,
                    semantics: ConnectionSemantics::Stream,
                }],
            },
        )
        .unwrap();
}

fn grants(paths: &[&[&str]]) -> MeshAccessPolicy {
    MeshAccessPolicy {
        grants: paths
            .iter()
            .enumerate()
            .map(|(i, path)| MeshAccessGrant {
                id: format!("g{i}"),
                path: path.iter().map(|id| (*id).into()).collect(),
            })
            .collect(),
    }
}

fn save(store: &mut Store, policy: &MeshAccessPolicy) -> Result<MeshAccessSnapshot, Error> {
    store.save_mesh_access(&SaveMeshAccessRequest {
        expected_revision: store.revision().unwrap(),
        policy: policy.clone(),
    })
}

#[test]
fn structural_errors_bound_diagnostic_bytes_and_skip_binding_validation() {
    for id_length in [120_000, MAX_MESH_ACCESS_BYTES] {
        let policy = MeshAccessPolicy {
            grants: vec![MeshAccessGrant {
                id: "x".repeat(id_length),
                path: vec!["missing".into(); 62],
            }],
        };
        let diagnostics = validate(&policy, &[], &BTreeMap::new());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule_id.is_none())
        );
        assert!(serde_json::to_vec(&diagnostics).unwrap().len() < 16 * 1024);
        if id_length == MAX_MESH_ACCESS_BYTES {
            assert!(serde_json::to_vec(&policy).unwrap().len() > MAX_MESH_ACCESS_BYTES);
            // Overflow must stop before the binding pass can add 63 more errors
            // containing copies of the oversized, structurally unchecked ID.
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].field, "policy");
        } else {
            assert_eq!(diagnostics.len(), meshway_policy::MAX_DIAGNOSTICS);
        }
    }
}

fn connected_plan(store: &mut Store, listener: &str, dialer: &str) {
    store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: store.revision().unwrap(),
            configuration: ConnectionPlan {
                listeners: vec![ConnectionListener {
                    id: "listener".into(),
                    node_id: listener.into(),
                    transport: ConnectionTransport::NativeTcp,
                    bind: "127.0.0.1:443".into(),
                    advertised_endpoints: vec![ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port: 443,
                    }],
                    credential_ref: None,
                }],
                candidates: vec![ConnectionCandidate {
                    id: "dial".into(),
                    dialer_node_id: dialer.into(),
                    listener_id: "listener".into(),
                    endpoint: ConnectionEndpoint {
                        host: "127.0.0.1".into(),
                        port: 443,
                    },
                    priority: 0,
                    credential_ref: None,
                }],
            },
        })
        .unwrap();
}

#[test]
fn saves_require_active_bound_identities_are_cas_idempotent_and_do_not_require_adjacency() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, _) = enrolled(&mut store, 9, true);
    let (b, token_b) = enrolled(&mut store, 10, false);
    let policy = grants(&[&[&a, &b]]);
    let before = store.revision().unwrap();
    assert!(matches!(
        save(&mut store, &policy),
        Err(Error::Unprocessable(_))
    ));
    assert_eq!(store.revision().unwrap(), before);
    assert!(store.mesh_access().unwrap().policy.grants.is_empty());
    bind(&mut store, &token_b, 10);
    let before = store.revision().unwrap();
    let saved = save(&mut store, &policy).unwrap();
    assert_eq!(saved.revision, before + 1);
    assert_eq!(save(&mut store, &policy).unwrap().revision, saved.revision);
    assert!(matches!(
        store.save_mesh_access(&SaveMeshAccessRequest {
            expected_revision: before,
            policy: policy.clone()
        }),
        Err(Error::Conflict)
    ));
    assert!(
        store
            .connections()
            .unwrap()
            .configuration
            .candidates
            .is_empty()
    );
    let snapshot = store.agent_dataplane(&token_b).unwrap();
    assert_eq!(snapshot.payload.access_policy, policy);
    assert!(snapshot.payload.connections.peers.is_empty());
    assert_eq!(snapshot.payload.identities[0].node_id, a);
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: saved.revision,
            policy: RoutingPolicy {
                default_action: DefaultRouteAction::Direct,
                rules: vec![],
            },
        })
        .unwrap();
    assert_eq!(store.mesh_access().unwrap().policy, policy);
    store
        .connection
        .execute(
            "UPDATE configuration SET revision = 4294967295 WHERE id = 1",
            [],
        )
        .unwrap();
    assert_eq!(save(&mut store, &policy).unwrap().revision, u32::MAX);
    assert!(matches!(
        save(&mut store, &MeshAccessPolicy::default()),
        Err(Error::Capacity(_))
    ));
    assert_eq!(store.mesh_access().unwrap().policy, policy);
}

#[test]
fn coherent_public_only_projection_retains_direction_and_e2e_identities_without_carrier_authority()
{
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, token_a) = enrolled(&mut store, 9, true);
    let (b, token_b) = enrolled(&mut store, 10, true);
    let (c, token_c) = enrolled(&mut store, 11, true);
    let (d, token_d) = enrolled(&mut store, 12, true);
    let (e, token_e) = enrolled(&mut store, 13, true);
    connected_plan(&mut store, &a, &d);
    let policy = grants(&[&[&a, &b, &c]]);
    save(&mut store, &policy).unwrap();
    let old_configuration =
        serde_json::to_value(store.configuration(&token_a).unwrap().payload).unwrap();
    let old_connections = store.agent_connections(&token_a).unwrap().payload;
    let snapshot = store.agent_dataplane(&token_a).unwrap();
    let payload = &snapshot.payload;
    assert_eq!(snapshot.digest, dataplane_digest(payload).unwrap());
    assert_eq!(
        snapshot.digest,
        store.agent_dataplane(&token_a).unwrap().digest
    );
    assert_eq!(payload.node_id, a);
    assert_eq!(payload.configuration.node.id, a);
    assert_eq!(payload.connections.node_id, a);
    assert_eq!(payload.revision, payload.configuration.revision);
    assert_eq!(payload.revision, payload.connections.revision);
    assert_eq!(payload.revision, store.mesh_access().unwrap().revision);
    assert_eq!(
        serde_json::to_value(&payload.configuration).unwrap(),
        old_configuration
    );
    assert_eq!(payload.connections, old_connections);
    assert_eq!(payload.access_policy, policy);
    assert_eq!(payload.configuration.peers.len(), 4);
    assert_eq!(payload.connections.peers.len(), 1);
    assert_eq!(payload.connections.peers[0].node_id, d);
    let ids: BTreeSet<_> = payload
        .identities
        .iter()
        .map(|peer| peer.node_id.as_str())
        .collect();
    assert_eq!(ids, BTreeSet::from([b.as_str(), c.as_str(), d.as_str()]));
    for token in [&token_b, &token_c] {
        let projected = store.agent_dataplane(token).unwrap().payload;
        assert_eq!(projected.access_policy, policy);
        assert_eq!(projected.identities.len(), 2);
        assert!(projected.connections.peers.is_empty());
    }
    for token in [&token_d, &token_e] {
        assert!(
            store
                .agent_dataplane(token)
                .unwrap()
                .payload
                .access_policy
                .grants
                .is_empty()
        );
    }
    assert!(
        store
            .agent_dataplane(&token_e)
            .unwrap()
            .payload
            .identities
            .is_empty()
    );
    let encoded = serde_json::to_string(&snapshot).unwrap();
    for secret in [&token_a, &token_b, &token_c, &token_d, &token_e] {
        assert!(!encoded.contains(secret));
    }
    for field in [
        "credential_hash",
        "private_key",
        "admin_hash",
        "enrollment_tokens",
    ] {
        assert!(!encoded.contains(field));
    }
    // Grant withdrawal and inventory revocation change the digest, immediately.
    save(&mut store, &MeshAccessPolicy::default()).unwrap();
    let withdrawn = store.agent_dataplane(&token_a).unwrap();
    assert_ne!(withdrawn.digest, snapshot.digest);
    assert!(withdrawn.payload.access_policy.grants.is_empty());
    assert_eq!(withdrawn.payload.identities, payload.connections.peers);
    assert!(payload.configuration.peers.iter().any(|node| node.id == e));
}

#[test]
fn revocation_withdraws_all_effective_grants_preserves_source_and_blocks_node_auth() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, token_a) = enrolled(&mut store, 9, true);
    let (b, token_b) = enrolled(&mut store, 10, true);
    let (c, _) = enrolled(&mut store, 11, true);
    let policy = grants(&[&[&a, &b], &[&a, &c]]);
    save(&mut store, &policy).unwrap();
    store.revoke(&b).unwrap();
    let source = store.mesh_access().unwrap();
    assert_eq!(source.policy, policy);
    assert!(!source.valid);
    assert!(!source.diagnostics.is_empty());
    let effective = store.agent_dataplane(&token_a).unwrap().payload;
    assert!(!effective.access_valid);
    assert!(effective.access_policy.grants.is_empty());
    assert!(effective.identities.is_empty());
    assert!(
        effective
            .configuration
            .peers
            .iter()
            .all(|node| node.id != b)
    );
    assert!(matches!(
        store.agent_dataplane(&token_b),
        Err(Error::Forbidden)
    ));
    let admin = crate::admin_token(dir.path()).unwrap();
    assert!(matches!(
        store.agent_dataplane(&admin),
        Err(Error::Unauthorized)
    ));
    let repaired = grants(&[&[&a, &c]]);
    save(&mut store, &repaired).unwrap();
    assert!(
        store
            .agent_dataplane(&token_a)
            .unwrap()
            .payload
            .access_valid
    );
    // Corrupted/missing bindings fail closed rather than minting an identity.
    store
        .connection
        .execute("DELETE FROM connection_identities WHERE node_id = ?1", [&c])
        .unwrap();
    let invalid = store.agent_dataplane(&token_a).unwrap().payload;
    assert!(!invalid.access_valid);
    assert!(invalid.access_policy.grants.is_empty());
    assert_eq!(store.mesh_access().unwrap().policy, repaired);
}

fn old_rows(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "settings",
        "nodes",
        "enrollment_tokens",
        "configuration",
        "connection_identities",
        "connection_source",
        "connection_receipts",
        "connection_runtime_leases",
        "connection_runtime_reports",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn additive_v4_upgrade_preserves_all_old_rows_keys_credentials_receipts_runtime_and_revision() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, token_a) = enrolled(&mut store, 9, true);
    let (b, _) = enrolled(&mut store, 10, true);
    connected_plan(&mut store, &a, &b);
    let config = store.configuration(&token_a).unwrap();
    store
        .acknowledge(
            &token_a,
            &AcknowledgeConfigurationRequest {
                revision: config.payload.revision,
                digest: config.digest,
            },
        )
        .unwrap();
    let connections = store.agent_connections(&token_a).unwrap();
    store
        .acknowledge_connections(
            &token_a,
            &AcknowledgeAgentConnectionsRequest {
                revision: connections.payload.revision,
                digest: connections.digest,
            },
        )
        .unwrap();
    let epoch = store
        .begin_connection_runtime(
            &token_a,
            &BeginConnectionRuntimeRequest {
                instance_id: "01".repeat(32),
                expected_epoch: 0,
            },
        )
        .unwrap()
        .epoch;
    let adjacency = store.agent_connections(&token_a).unwrap();
    store
        .report_connection_runtime(
            &token_a,
            &ReportConnectionRuntimeRequest {
                epoch,
                sequence: 1,
                revision: adjacency.payload.revision,
                digest: adjacency.digest,
                state: ConnectionRuntimeState::Disabled,
                reason: ConnectionRuntimeReason::Disabled,
                listeners: vec![],
                peers: vec![],
            },
        )
        .unwrap();
    let unused = store.create_token(3600).unwrap();
    store.revoke(&b).unwrap();
    let before = old_rows(&store.connection);
    let admin = std::fs::read(dir.path().join("admin.token")).unwrap();
    let old_configuration = serde_json::to_vec(&store.configuration(&token_a).unwrap()).unwrap();
    let old_connections = serde_json::to_vec(&store.agent_connections(&token_a).unwrap()).unwrap();
    store
        .connection
        .execute_batch("DROP TABLE mesh_access_source; PRAGMA user_version=4;")
        .unwrap();
    drop(store);
    let mut reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(old_rows(&reopened.connection), before);
    assert_eq!(
        std::fs::read(dir.path().join("admin.token")).unwrap(),
        admin
    );
    assert_eq!(
        serde_json::to_vec(&reopened.configuration(&token_a).unwrap()).unwrap(),
        old_configuration
    );
    assert_eq!(
        serde_json::to_vec(&reopened.agent_connections(&token_a).unwrap()).unwrap(),
        old_connections
    );
    assert_eq!(
        reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        5
    );
    let empty = reopened.mesh_access().unwrap();
    assert!(empty.valid);
    assert_eq!(empty.policy, MeshAccessPolicy::default());
    let dataplane = reopened.agent_dataplane(&token_a).unwrap();
    assert!(dataplane.payload.access_policy.grants.is_empty());
    assert_eq!(dataplane.payload.revision, empty.revision);
    reopened
        .enroll(
            &unused.token,
            &EnrollAgentRequest {
                node_credential: local::credential().unwrap(),
                name: "after-upgrade".into(),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    drop(reopened);
    assert!(
        Store::open(dir.path(), &NetworkOptions::default())
            .unwrap()
            .mesh_access()
            .unwrap()
            .valid
    );
}

#[test]
fn identity_union_is_bounded_at_save_and_get_without_truncation() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, token_a) = enrolled(&mut store, 9, true);
    // Direct test fixtures avoid repeatedly publishing an entire growing node
    // inventory. Each key is still checked by the real save/snapshot validator.
    let key = format!("0a{}", "00".repeat(31));
    let mut peer_ids = Vec::new();
    for i in 1..=65_u16 {
        let id = format!("peer-{i}");
        let public_key = format!("{i:02x}01{}", "00".repeat(30));
        assert!(valid_connection_public_key(&public_key));
        store.connection.execute("INSERT INTO nodes (id,credential_hash,name,agent_version,registered_at,last_seen,mesh_ipv4,mesh_ipv6) VALUES (?1,?2,?1,'test','2026-09-30T00:00:00Z','2026-09-30T00:00:00Z',?3,?4)", params![id, local::digest(&id), format!("100.96.0.{}", i+1), format!("{}{:x}", store.policy().unwrap().network.ipv6_prefix.trim_end_matches("/64"), i+1)]).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO connection_identities VALUES (?1,?2,'[]')",
                params![id, public_key],
            )
            .unwrap();
        peer_ids.push(id);
    }
    assert!(valid_connection_public_key(&key));
    let policy = MeshAccessPolicy {
        grants: peer_ids
            .iter()
            .enumerate()
            .map(|(i, id)| MeshAccessGrant {
                id: format!("grant-{i}"),
                path: vec![a.clone(), id.clone()],
            })
            .collect(),
    };
    let mut bounded = policy.clone();
    bounded.grants.pop();
    save(&mut store, &bounded).unwrap();
    assert_eq!(
        store
            .agent_dataplane(&token_a)
            .unwrap()
            .payload
            .identities
            .len(),
        64
    );
    let revision = store.revision().unwrap();
    assert!(matches!(save(&mut store, &policy), Err(Error::Capacity(_))));
    assert_eq!(store.revision().unwrap(), revision);
    // A previously valid access source combined with later adjacency can grow
    // the union. Simulate that projection through the shared union validator.
    let mut adjacency = store.agent_connections(&token_a).unwrap().payload;
    adjacency.peers.push(ConnectionPeer {
        node_id: peer_ids[64].clone(),
        public_key: key,
    });
    assert!(matches!(
        identities(
            &a,
            &adjacency,
            &bounded,
            &bindings(&store.connection).unwrap()
        ),
        Err(Error::Capacity(_))
    ));
    store
        .connection
        .execute(
            "UPDATE mesh_access_source SET policy = ?1",
            [serde_json::to_string(&policy).unwrap()],
        )
        .unwrap();
    assert!(matches!(
        store.agent_dataplane(&token_a),
        Err(Error::Capacity(_))
    ));
}
