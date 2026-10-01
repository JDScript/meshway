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

fn enrolled(store: &mut Store, seed: u8, bind: bool) -> (String, String) {
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
    if bind {
        store
            .publish_connection_identity(
                &credential,
                &PublishConnectionIdentityRequest {
                    public_key: format!("{seed:02x}{}", "00".repeat(31)),
                    capabilities: vec![],
                },
            )
            .unwrap();
    }
    (node.id, credential)
}

fn direct(store: &mut Store) {
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: store.revision().unwrap(),
            policy: RoutingPolicy {
                default_action: DefaultRouteAction::Direct,
                rules: vec![],
            },
            source_jsonc: None,
        })
        .unwrap();
}

fn rule(
    id: &str,
    source: Vec<String>,
    destination: Vec<String>,
    action: AccessAction,
    bidirectional: bool,
) -> MeshAccessRule {
    MeshAccessRule {
        id: id.into(),
        source,
        destination,
        action,
        bidirectional,
    }
}

fn save(store: &mut Store, policy: MeshAccessPolicy) -> MeshAccessSnapshot {
    store
        .save_mesh_access(&SaveMeshAccessRequest {
            expected_revision: store.revision().unwrap(),
            policy,
            source_jsonc: None,
        })
        .unwrap()
}

#[test]
fn ordered_first_match_tags_bidirectional_and_keyless_nodes_compile_exact_paths() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, token_a) = enrolled(&mut store, 9, true);
    let (b, _) = enrolled(&mut store, 10, true);
    let (c, token_c) = enrolled(&mut store, 11, true);
    let (_keyless, keyless_token) = enrolled(&mut store, 12, false);
    direct(&mut store);
    store
        .save_tags(&SaveTagsRequest {
            expected_revision: store.revision().unwrap(),
            tags: vec![TagDefinition {
                tag: "tag:prod".into(),
                description: "production".into(),
            }],
        })
        .unwrap();
    for id in [&a, &b] {
        store
            .set_node_tags(&SetNodeTagsRequest {
                expected_revision: store.revision().unwrap(),
                node_id: id.clone(),
                tags: vec!["tag:prod".into()],
            })
            .unwrap();
    }
    let policy = MeshAccessPolicy {
        default_action: AccessAction::Reject,
        rules: vec![
            rule(
                "reject-a-b",
                vec![format!("node:{a}")],
                vec![format!("node:{b}")],
                AccessAction::Reject,
                false,
            ),
            rule(
                "prod-c",
                vec!["tag:prod".into()],
                vec![format!("node:{c}")],
                AccessAction::Allow,
                true,
            ),
        ],
    };
    save(&mut store, policy);
    let revision = store.revision().unwrap();
    assert!(matches!(
        store.save_tags(&SaveTagsRequest {
            expected_revision: revision,
            tags: vec![],
        }),
        Err(Error::Unprocessable(_))
    ));
    assert_eq!(store.revision().unwrap(), revision);
    assert!(
        store
            .agent_dataplane(&token_a)
            .unwrap()
            .payload
            .access_policy
            .grants
            .iter()
            .all(|grant| grant.path != [a.clone(), b.clone()])
    );
    let c_paths = store
        .agent_dataplane(&token_c)
        .unwrap()
        .payload
        .access_policy
        .grants;
    assert_eq!(c_paths.len(), 4);
    assert!(
        c_paths
            .iter()
            .any(|grant| grant.path == [a.clone(), c.clone()])
    );
    assert!(
        c_paths
            .iter()
            .any(|grant| grant.path == [c.clone(), a.clone()])
    );
    assert!(
        store
            .agent_dataplane(&keyless_token)
            .unwrap()
            .payload
            .access_policy
            .grants
            .is_empty()
    );

    let routed = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![RouteRule {
            id: "route-a-c".into(),
            source: SourceSelector {
                node_ids: vec![a.clone()],
                countries: vec![],
            },
            destination_scope: DestinationScope::Mesh,
            destination_any: vec![DestinationSelector {
                kind: DestinationSelectorKind::Node,
                value: c.clone(),
            }],
            action: RouteAction {
                kind: RouteActionKind::Route,
                via: vec![b.clone()],
                exit_node_id: None,
            },
        }],
    };
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: store.revision().unwrap(),
            policy: routed.clone(),
            source_jsonc: None,
        })
        .unwrap();
    let transit = store
        .agent_dataplane(&token_a)
        .unwrap()
        .payload
        .access_policy
        .grants;
    assert!(
        transit
            .iter()
            .any(|grant| grant.path == [a.clone(), b.clone(), c.clone()])
    );
    let mut rejected = routed;
    rejected.rules[0].action = RouteAction {
        kind: RouteActionKind::Reject,
        via: vec![],
        exit_node_id: None,
    };
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: store.revision().unwrap(),
            policy: rejected,
            source_jsonc: None,
        })
        .unwrap();
    assert!(
        store
            .agent_dataplane(&token_a)
            .unwrap()
            .payload
            .access_policy
            .grants
            .iter()
            .all(|grant| grant.path != [a.clone(), b.clone(), c.clone()])
    );
    store.revoke(&c).unwrap();
    let invalid = store.mesh_access().unwrap();
    assert!(!invalid.valid);
    let withdrawn = store.agent_dataplane(&token_a).unwrap().payload;
    assert!(!withdrawn.access_valid);
    assert!(withdrawn.access_policy.grants.is_empty());
}

#[test]
fn jsonc_is_strict_equal_atomic_and_comment_text_is_a_cas_change() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let source = "{ // retained\n \"default_action\": \"reject\", \"rules\": [], }";
    let policy = MeshAccessPolicy::default();
    let before = store.revision().unwrap();
    let saved = store
        .save_mesh_access(&SaveMeshAccessRequest {
            expected_revision: before,
            policy: policy.clone(),
            source_jsonc: Some(source.into()),
        })
        .unwrap();
    assert_eq!(saved.source_jsonc.as_deref(), Some(source));
    assert_eq!(saved.revision, before + 1);
    assert_eq!(
        store
            .save_mesh_access(&SaveMeshAccessRequest {
                expected_revision: saved.revision,
                policy: policy.clone(),
                source_jsonc: Some(source.into()),
            })
            .unwrap()
            .revision,
        saved.revision
    );
    for invalid in [
        "{\"default_action\":\"reject\",\"default_action\":\"reject\",\"rules\":[]}",
        "{\"default_action\":\"allow\",\"rules\":[]}",
    ] {
        assert!(matches!(
            store.save_mesh_access(&SaveMeshAccessRequest {
                expected_revision: saved.revision,
                policy: policy.clone(),
                source_jsonc: Some(invalid.into()),
            }),
            Err(Error::Unprocessable(_))
        ));
        assert_eq!(store.revision().unwrap(), saved.revision);
    }
}

#[test]
fn routing_jsonc_preserves_exact_admin_policy_but_agents_receive_normalized_projection() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (_, token) = enrolled(&mut store, 9, false);
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Reject,
        rules: vec![RouteRule {
            id: "country".into(),
            source: SourceSelector {
                node_ids: vec![],
                countries: vec!["us".into()],
            },
            destination_scope: DestinationScope::Any,
            destination_any: vec![],
            action: RouteAction {
                kind: RouteActionKind::Reject,
                via: vec![],
                exit_node_id: None,
            },
        }],
    };
    let source = format!(
        "// retained routing source\n{}",
        serde_json::to_string(&policy).unwrap()
    );
    let before = store.revision().unwrap();
    let snapshot = store
        .save_policy(&SavePolicyRequest {
            expected_revision: before,
            policy: policy.clone(),
            source_jsonc: Some(source.clone()),
        })
        .unwrap();
    assert_eq!(snapshot.policy, policy);
    assert_eq!(snapshot.source_jsonc.as_deref(), Some(source.as_str()));
    assert_eq!(
        store
            .configuration(&token)
            .unwrap()
            .payload
            .routing_policy
            .rules[0]
            .source
            .countries,
        ["US"]
    );
    assert!(matches!(
        store.save_policy(&SavePolicyRequest {
            expected_revision: snapshot.revision,
            policy: snapshot.policy.clone(),
            source_jsonc: Some(
                "{\"default_action\":\"reject\",\"default_action\":\"reject\",\"rules\":[]}".into()
            ),
        }),
        Err(Error::Unprocessable(_))
    ));
    assert_eq!(store.revision().unwrap(), snapshot.revision);
}

#[test]
fn actual_v5_migration_deduplicates_paths_folds_reverse_and_bumps_revision() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let (a, _) = enrolled(&mut store, 9, false);
    let (b, _) = enrolled(&mut store, 10, false);
    let legacy = MeshPathAuthorization {
        grants: vec![
            MeshAccessGrant {
                id: "z-forward".into(),
                path: vec![a.clone(), b.clone()],
            },
            MeshAccessGrant {
                id: "a-reverse".into(),
                path: vec![b.clone(), a.clone()],
            },
            MeshAccessGrant {
                id: "duplicate-via".into(),
                path: vec![a.clone(), "via".into(), b.clone()],
            },
        ],
    };
    let revision = store.revision().unwrap();
    store
        .connection
        .execute(
            "UPDATE mesh_access_legacy_v5 SET policy=?1",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();
    store.connection.execute_batch("DROP TABLE mesh_access_source; ALTER TABLE mesh_access_legacy_v5 RENAME TO mesh_access_source; DROP TABLE node_tags; DROP TABLE tag_definitions; ALTER TABLE configuration DROP COLUMN source_jsonc; PRAGMA user_version=5;").unwrap();
    drop(store);
    let reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let snapshot = reopened.mesh_access().unwrap();
    assert_eq!(snapshot.revision, revision + 1);
    assert_eq!(snapshot.policy.default_action, AccessAction::Reject);
    assert_eq!(snapshot.policy.rules.len(), 1);
    assert_eq!(snapshot.policy.rules[0].id, "a-reverse");
    assert!(snapshot.policy.rules[0].bidirectional);
    assert_eq!(
        reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        6
    );
}

#[test]
fn node_dataplane_keeps_the_v1_access_json_shape() {
    let value = serde_json::to_value(NodeDataplane {
        schema_version: 1,
        revision: 1,
        node_id: "node".into(),
        configuration: serde_json::from_value(serde_json::json!({
            "schema_version":1,"revision":1,
            "network":{"ipv4_prefix":"100.96.0.0/11","ipv6_prefix":"fd00::/64"},
            "node":{"id":"node","name":"node","country":null,"mesh_ipv4":"100.96.0.1","mesh_ipv6":"fd00::1"},
            "peers":[],"routing_policy":{"default_action":"reject","rules":[]},"policy_valid":true,"diagnostics":[]
        })).unwrap(),
        connections: NodeConnections { schema_version: 1, revision: 1, node_id: "node".into(), public_key: None, capabilities: vec![], valid: true, diagnostics: vec![], listeners: vec![], outgoing: vec![], incoming: vec![], peers: vec![] },
        access_policy: MeshPathAuthorization::default(),
        access_valid: true,
        identities: vec![],
    }).unwrap();
    assert_eq!(value["access_policy"], serde_json::json!({"grants":[]}));
    assert!(value["access_policy"].get("rules").is_none());
}

#[test]
fn wildcard_expansion_rejects_identity_overflow_without_truncation_or_revision_change() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    direct(&mut store);
    let ipv6 = store.policy().unwrap().network.ipv6_prefix;
    let ipv6 = ipv6.trim_end_matches("/64");
    for i in 1..=66_u32 {
        let id = format!("{i:064x}");
        let public_key = format!("{i:02x}01{}", "00".repeat(30));
        assert!(valid_connection_public_key(&public_key));
        store.connection.execute(
            "INSERT INTO nodes(id,credential_hash,name,agent_version,registered_at,last_seen,mesh_ipv4,mesh_ipv6) VALUES(?1,?2,?1,'test','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z',?3,?4)",
            params![id, local::digest(&format!("credential-{i}")), format!("100.96.0.{i}"), format!("{ipv6}{i:x}")],
        ).unwrap();
        store.connection.execute(
            "INSERT INTO connection_identities(node_id,public_key,capabilities) VALUES(?1,?2,'[]')",
            params![id, public_key],
        ).unwrap();
    }
    let routing = store.policy().unwrap();
    assert!(routing.valid, "{:?}", routing.diagnostics);
    let revision = store.revision().unwrap();
    let policy = MeshAccessPolicy {
        default_action: AccessAction::Reject,
        rules: vec![rule(
            "all",
            vec!["*".into()],
            vec!["*".into()],
            AccessAction::Allow,
            false,
        )],
    };
    let saved = store.save_mesh_access(&SaveMeshAccessRequest {
        expected_revision: revision,
        policy,
        source_jsonc: None,
    });
    let error = saved.unwrap_err();
    assert!(matches!(error, Error::Capacity(_)), "{error:?}");
    assert_eq!(store.revision().unwrap(), revision);
    assert_eq!(
        store.mesh_access().unwrap().policy,
        MeshAccessPolicy::default()
    );
}
