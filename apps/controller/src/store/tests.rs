use meshway_protocol::*;
use rusqlite::params;

use super::Store;
use crate::{Error, NetworkOptions};

pub(super) fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

pub(super) fn request(name: &str) -> EnrollAgentRequest {
    EnrollAgentRequest {
        node_credential: local::credential().unwrap(),
        name: name.into(),
        agent_version: "v1".into(),
    }
}

pub(super) fn enroll(store: &mut Store, request: &EnrollAgentRequest) -> NodeSummary {
    let token = store.create_token(3600).unwrap();
    store.enroll(&token.token, request).unwrap()
}

#[test]
fn device_login_v8_migration_preserves_existing_rows_and_reopens() {
    let dir = directory();
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    let enrollment = request("existing");
    let node = enroll(&mut store, &enrollment);
    store
        .publish_connection_identity(
            &enrollment.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: "09".repeat(32),
                capabilities: vec![],
            },
        )
        .unwrap();
    let current = store.policy().unwrap();
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: current.revision,
            policy: RoutingPolicy {
                default_action: DefaultRouteAction::Direct,
                ..current.policy
            },
            source_jsonc: None,
        })
        .unwrap();
    let token = store.create_token(3600).unwrap();
    store
        .connection
        .execute_batch(
            "ALTER TABLE enrollment_tokens DROP COLUMN device_binding; PRAGMA user_version=7;",
        )
        .unwrap();
    fn snapshot(
        connection: &rusqlite::Connection,
    ) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let tables: Vec<String> = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        tables
            .into_iter()
            .map(|table| {
                let columns: Vec<String> = connection
                    .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                    .unwrap()
                    .query_map([], |r| r.get(1))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                let columns: Vec<_> = columns
                    .into_iter()
                    .filter(|c| c != "device_binding")
                    .collect();
                let sql = format!(
                    "SELECT {} FROM \"{table}\" ORDER BY rowid",
                    columns
                        .iter()
                        .map(|c| format!("\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                let rows = connection
                    .prepare(&sql)
                    .unwrap()
                    .query_map([], |row| {
                        (0..columns.len())
                            .map(|i| row.get(i))
                            .collect::<Result<Vec<rusqlite::types::Value>, _>>()
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                (table, rows)
            })
            .collect()
    }
    let before = snapshot(&store.connection);
    drop(store);
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        8
    );
    assert_eq!(snapshot(&store.connection), before);
    assert_eq!(
        store
            .heartbeat(&enrollment.node_credential, "v1")
            .unwrap()
            .id,
        node.id
    );
    assert!(store.enroll(&token.token, &request("new")).is_ok());
    drop(store);
    assert!(Store::open(dir.path(), &NetworkOptions::default()).is_ok());
}

#[test]
fn dualstack_allocation_revision_and_exhaustion_are_transactional() {
    let dir = directory();
    let options = NetworkOptions {
        ipv4_prefix: Some("100.96.0.0/30".into()),
        ipv6_prefix: Some("fd12:3456:789a::/64".into()),
    };
    let mut store = Store::open(dir.path(), &options).unwrap();
    let initial = store.policy().unwrap();
    assert_eq!(initial.revision, 1);
    assert_eq!(initial.policy, RoutingPolicy::default());
    let request = request("one");
    let token = store.create_token(3600).unwrap().token;
    assert_eq!(store.revision().unwrap(), 1);
    let first = store.enroll(&token, &request).unwrap();
    assert_eq!(first.mesh_ipv4, "100.96.0.1");
    assert_eq!(first.mesh_ipv6, "fd12:3456:789a::1");
    assert_eq!(first.configuration_revision, 2);
    assert_eq!(
        store
            .enroll(&token, &request)
            .unwrap()
            .configuration_revision,
        2
    );
    assert_eq!(
        store
            .heartbeat(&request.node_credential, "v2")
            .unwrap()
            .configuration_revision,
        2
    );
    let second = enroll(&mut store, &self::request("two"));
    assert_eq!(second.mesh_ipv4, "100.96.0.2");
    assert_eq!(second.mesh_ipv6, "fd12:3456:789a::2");
    assert_eq!(store.revoke(&first.id).unwrap().configuration_revision, 4);
    assert_eq!(store.revoke(&first.id).unwrap().configuration_revision, 4);
    let unused = store.create_token(3600).unwrap().token;
    let third = self::request("three");
    assert!(matches!(
        store.enroll(&unused, &third),
        Err(Error::Capacity(_))
    ));
    let binding: Option<String> = store
        .connection
        .query_row(
            "SELECT used_node_id FROM enrollment_tokens WHERE token_hash = ?1",
            [local::digest(&unused)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(binding, None);
    assert_eq!(store.count().unwrap(), 2);
    assert_eq!(store.revision().unwrap(), 4);
    assert!(matches!(
        store.enroll(&unused, &third),
        Err(Error::Capacity(_))
    ));
    let before = serde_json::to_value(store.nodes().unwrap()).unwrap();
    drop(store);
    let reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(reopened.policy().unwrap().network, initial.network);
    assert_eq!(
        serde_json::to_value(reopened.nodes().unwrap()).unwrap(),
        before
    );
    drop(reopened);
    assert!(
        Store::open(
            dir.path(),
            &NetworkOptions {
                ipv4_prefix: Some("100.97.0.0/16".into()),
                ipv6_prefix: None
            }
        )
        .is_err()
    );
    assert!(Store::open(dir.path(), &options).is_ok());
}

#[test]
fn policy_cas_country_and_node_scoped_receipts_preserve_revision_semantics() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let one = request("one");
    let two = request("two");
    let source = enroll(&mut store, &one);
    let peer = enroll(&mut store, &two);
    let snapshot = store.configuration(&one.node_credential).unwrap();
    assert_eq!(snapshot.payload.revision, 3);
    assert_eq!(snapshot.payload.node.id, source.id);
    assert_eq!(snapshot.payload.peers[0].id, peer.id);
    assert_ne!(
        store.configuration(&two.node_credential).unwrap().digest,
        snapshot.digest
    );
    let encoded = serde_json::to_string(&snapshot).unwrap();
    for secret in [&one.node_credential, &two.node_credential] {
        assert!(!encoded.contains(secret));
    }
    for transient in [
        "last_seen",
        "agent_version",
        "acknowledged",
        "credential",
        "revoked_at",
    ] {
        assert!(!encoded.contains(transient));
    }
    for (revision, digest) in [
        (2, snapshot.digest.clone()),
        (4, snapshot.digest.clone()),
        (3, "0".repeat(64)),
        (3, store.configuration(&two.node_credential).unwrap().digest),
    ] {
        assert!(matches!(
            store.acknowledge(
                &one.node_credential,
                &AcknowledgeConfigurationRequest { revision, digest }
            ),
            Err(Error::Conflict)
        ));
    }
    let ack = AcknowledgeConfigurationRequest {
        revision: 3,
        digest: snapshot.digest.clone(),
    };
    store.acknowledge(&one.node_credential, &ack).unwrap();
    store.acknowledge(&one.node_credential, &ack).unwrap();
    store
        .heartbeat(&one.node_credential, "transient-new-version")
        .unwrap();
    assert_eq!(store.revision().unwrap(), 3);
    assert_eq!(
        store.configuration(&one.node_credential).unwrap().digest,
        snapshot.digest
    );
    assert_eq!(
        super::find_node(&store.connection, &source.id)
            .unwrap()
            .acknowledged_revision,
        3
    );
    assert_eq!(
        super::find_node(&store.connection, &peer.id)
            .unwrap()
            .acknowledged_revision,
        0
    );

    let country = SetNodeCountryRequest {
        node_id: source.id.clone(),
        country: Some("us".into()),
        expected_revision: 3,
    };
    let changed = store.set_country(&country).unwrap();
    assert_eq!(changed.country.as_deref(), Some("US"));
    assert_eq!(changed.configuration_revision, 4);
    assert!(matches!(store.set_country(&country), Err(Error::Conflict)));
    assert_eq!(
        store
            .set_country(&SetNodeCountryRequest {
                expected_revision: 4,
                ..country
            })
            .unwrap()
            .configuration_revision,
        4
    );

    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Direct,
        rules: vec![RouteRule {
            id: "through-peer".into(),
            source: SourceSelector::default(),
            destination_scope: DestinationScope::Mesh,
            destination_any: vec![DestinationSelector {
                kind: DestinationSelectorKind::DomainSuffix,
                value: "ExAmPlE.COM.".into(),
            }],
            action: RouteAction {
                kind: RouteActionKind::Route,
                via: vec![peer.id.clone()],
                exit_node_id: None,
            },
        }],
    };
    let preview = store
        .preview(&PreviewPolicyRequest {
            policy: policy.clone(),
            flow: FlowQuery {
                source_node_id: source.id.clone(),
                destination_node_id: None,
                destination_ip: None,
                domain: Some("example.com".into()),
                destination_country: None,
            },
        })
        .unwrap();
    assert!(preview.valid);
    assert_eq!(store.revision().unwrap(), 4);
    assert_eq!(store.policy().unwrap().policy, RoutingPolicy::default());
    let saved = store
        .save_policy(&SavePolicyRequest {
            expected_revision: 4,
            policy: policy.clone(),
            source_jsonc: None,
        })
        .unwrap();
    assert_eq!(saved.revision, 5);
    assert_eq!(
        saved.policy.rules[0].destination_any[0].value,
        "example.com"
    );
    assert!(matches!(
        store.save_policy(&SavePolicyRequest {
            expected_revision: 4,
            policy: policy.clone(),
            source_jsonc: None,
        }),
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .save_policy(&SavePolicyRequest {
                expected_revision: 5,
                policy,
                source_jsonc: None,
            })
            .unwrap()
            .revision,
        5
    );
    let mut invalid = saved.policy.clone();
    invalid.rules[0].action.via = vec!["missing".into()];
    assert!(matches!(
        store.save_policy(&SavePolicyRequest {
            expected_revision: 5,
            policy: invalid,
            source_jsonc: None,
        }),
        Err(Error::Unprocessable(_))
    ));
    assert_eq!(store.revision().unwrap(), 5);
    store.revoke(&peer.id).unwrap();
    let source_policy = store.policy().unwrap();
    assert!(!source_policy.valid);
    assert_eq!(source_policy.policy, saved.policy);
    let effective = store.configuration(&one.node_credential).unwrap();
    assert!(!effective.payload.policy_valid);
    assert_eq!(effective.payload.revision, 6);
    assert_eq!(effective.payload.routing_policy, RoutingPolicy::default());
    assert!(!effective.payload.diagnostics.is_empty());
    assert!(effective.payload.peers.is_empty());
    assert!(matches!(
        store.configuration(&two.node_credential),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.acknowledge(&two.node_credential, &ack),
        Err(Error::Forbidden)
    ));
    drop(store);
    let reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(
        reopened.configuration(&one.node_credential).unwrap().digest,
        effective.digest
    );
    assert_eq!(
        super::find_node(&reopened.connection, &source.id)
            .unwrap()
            .acknowledged_revision,
        3
    );
}

/// Construct an actual historical v1 database, not a v2 database with its
/// version number changed. Token bindings include active and revoked nodes.
fn legacy(path: &std::path::Path) -> (String, Vec<EnrollAgentRequest>, Vec<String>) {
    let admin = local::credential().unwrap();
    local::atomic_write(&path.join("admin.token"), admin.as_bytes(), false).unwrap();
    local::atomic_write(&path.join("controller.sqlite3"), &[], false).unwrap();
    let db = rusqlite::Connection::open(path.join("controller.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), admin_hash BLOB NOT NULL);
        CREATE TABLE nodes (id TEXT PRIMARY KEY, credential_hash BLOB NOT NULL UNIQUE, name TEXT NOT NULL, agent_version TEXT NOT NULL, registered_at TEXT NOT NULL, last_seen TEXT NOT NULL, revoked_at TEXT);
        CREATE TABLE enrollment_tokens (id TEXT PRIMARY KEY, token_hash BLOB NOT NULL UNIQUE, expires_at INTEGER NOT NULL, used_node_id TEXT REFERENCES nodes(id), binding_hash BLOB, CHECK ((used_node_id IS NULL) = (binding_hash IS NULL)));
        PRAGMA user_version = 1;").unwrap();
    db.execute(
        "INSERT INTO settings VALUES (1, ?1)",
        [local::digest(&admin)],
    )
    .unwrap();
    let mut requests = Vec::new();
    let mut tokens = Vec::new();
    for (id, revoked) in [
        ("z", None),
        ("a", Some("2026-09-30T00:00:00Z")),
        ("m", None),
    ] {
        let request = request(id);
        let token = local::credential().unwrap();
        db.execute("INSERT INTO nodes VALUES (?1, ?2, ?1, 'v1', '2026-09-30T00:00:00Z', '2026-09-30T00:00:00Z', ?3)", params![id, local::digest(&request.node_credential), revoked]).unwrap();
        db.execute(
            "INSERT INTO enrollment_tokens VALUES (?1, ?2, 0, ?1, ?3)",
            params![
                id,
                local::digest(&token),
                local::digest(&serde_json::to_string(&request).unwrap())
            ],
        )
        .unwrap();
        requests.push(request);
        tokens.push(token);
    }
    (admin, requests, tokens)
}

#[test]
fn additive_v1_upgrade_preserves_binding_revocation_and_rolls_back_on_exhaustion() {
    let dir = directory();
    let (admin, requests, tokens) = legacy(dir.path());
    let small = NetworkOptions {
        ipv4_prefix: Some("100.96.0.0/30".into()),
        ipv6_prefix: None,
    };
    assert!(matches!(
        Store::open(dir.path(), &small),
        Err(Error::Capacity(_))
    ));
    let db = rusqlite::Connection::open(dir.path().join("controller.sqlite3")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert!(db.prepare("SELECT mesh_ipv4 FROM nodes").is_err());
    assert!(db.prepare("SELECT * FROM configuration").is_err());
    drop(db);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    store.authorize_admin(&admin).unwrap();
    let nodes = store.nodes().unwrap();
    assert_eq!(
        nodes
            .iter()
            .map(|node| (node.id.as_str(), node.mesh_ipv4.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("a", "100.96.0.1"),
            ("m", "100.96.0.2"),
            ("z", "100.96.0.3")
        ]
    );
    assert!(nodes[0].revoked_at.is_some());
    assert_eq!(store.enroll(&tokens[0], &requests[0]).unwrap().id, "z");
    assert!(matches!(
        store.enroll(&tokens[1], &requests[1]),
        Err(Error::Forbidden)
    ));
    assert_eq!(
        store
            .heartbeat(&requests[2].node_credential, "new")
            .unwrap()
            .id,
        "m"
    );
    assert_eq!(store.revision().unwrap(), 1);
    let network = store.policy().unwrap().network;
    drop(store);
    let reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(reopened.policy().unwrap().network, network);
    assert_eq!(
        reopened
            .nodes()
            .unwrap()
            .iter()
            .map(|node| &node.mesh_ipv6)
            .collect::<Vec<_>>(),
        nodes.iter().map(|node| &node.mesh_ipv6).collect::<Vec<_>>()
    );
}

#[test]
fn additive_v2_to_v3_preserves_configuration_credentials_addresses_and_receipts() {
    let dir = directory();
    let (admin, requests, tokens) = legacy(dir.path());
    let mut db = rusqlite::Connection::open(dir.path().join("controller.sqlite3")).unwrap();
    super::ipam::upgrade(&mut db, &NetworkOptions::default()).unwrap();
    db.execute("UPDATE configuration SET revision = 17, policy = '{\"default_action\":\"direct\",\"rules\":[]}'", []).unwrap();
    db.execute("UPDATE nodes SET country = 'US', acknowledged_revision = 16, acknowledged_digest = ?1 WHERE id = 'z'", ["ab".repeat(32)]).unwrap();
    let receipt_digest = |connection: &rusqlite::Connection| {
        connection
            .query_row(
                "SELECT CAST(acknowledged_digest AS BLOB) FROM nodes WHERE id = 'z'",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap()
    };
    let before_digest = receipt_digest(&db);
    let before = db
        .prepare(super::NODE_SELECT)
        .unwrap()
        .query_map([], super::node_row)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let token_bytes = std::fs::read(dir.path().join("admin.token")).unwrap();
    let network = super::ipam::network(&db).unwrap();
    drop(db);
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(receipt_digest(&store.connection), before_digest);
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        std::fs::read(dir.path().join("admin.token")).unwrap(),
        token_bytes
    );
    store.authorize_admin(&admin).unwrap();
    let after = store
        .connection
        .prepare(super::NODE_SELECT)
        .unwrap()
        .query_map([], super::node_row)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    assert_eq!(store.policy().unwrap().network, network);
    assert_eq!(
        store.policy().unwrap().policy.default_action,
        DefaultRouteAction::Direct
    );
    let initial = store.connections().unwrap();
    assert_eq!(initial.revision, 17);
    assert!(initial.valid && initial.configuration == ConnectionPlan::default());
    assert!(
        initial
            .nodes
            .iter()
            .all(|node| node.public_key.is_none() && node.acknowledged_revision == 0)
    );
    assert_eq!(store.enroll(&tokens[0], &requests[0]).unwrap().id, "z");
    assert!(matches!(
        store.enroll(&tokens[1], &requests[1]),
        Err(Error::Forbidden)
    ));
    assert_eq!(store.enroll(&tokens[2], &requests[2]).unwrap().id, "m");
    drop(store);
    let reopened = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(receipt_digest(&reopened.connection), before_digest);
    reopened.authorize_admin(&admin).unwrap();
    assert_eq!(reopened.revision().unwrap(), 17);
    assert!(reopened.connections().unwrap().valid);
    assert_eq!(
        reopened
            .configuration(&requests[0].node_credential)
            .unwrap()
            .payload
            .network,
        network
    );
}
