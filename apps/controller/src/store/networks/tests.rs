use meshway_protocol::*;
use rusqlite::{Connection, params, types::Value};

use super::{TABLES, install_scope, upgrade};
use crate::{
    Error, NetworkOptions,
    store::{
        Store,
        tests::{directory, enroll, request},
    },
};

fn rows(connection: &Connection, scoped: bool) -> Vec<(String, Vec<Vec<Value>>)> {
    TABLES
        .iter()
        .map(|table| {
            let physical = if scoped {
                format!("{table}_scoped")
            } else {
                (*table).to_owned()
            };
            let columns = connection
                .prepare(&format!("PRAGMA main.table_info({physical})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let names = columns
                .into_iter()
                .filter(|name| name != "network_id" || *table == "network_identity")
                .collect::<Vec<_>>();
            let mut query = connection
                .prepare(&format!(
                    "SELECT {} FROM main.{physical} ORDER BY 1,2",
                    names.join(",")
                ))
                .unwrap();
            let values = query
                .query_map([], |row| {
                    (0..names.len())
                        .map(|i| row.get::<_, Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            ((*table).to_owned(), values)
        })
        .collect()
}

#[test]
fn complete_v8_upgrade_is_lossless_and_rolls_back_on_invalid_ownership() {
    let dir = directory();
    let mut store = Store::open_v8(
        dir.path(),
        &NetworkOptions {
            ipv4_prefix: Some("100.96.0.0/16".into()),
            ipv6_prefix: Some("fd73:96a6:cf5e::/64".into()),
        },
    )
    .unwrap();
    let first = request("existing");
    let node = enroll(&mut store, &first);
    let second = request("revoked");
    let revoked = enroll(&mut store, &second);
    store.revoke(&revoked.id).unwrap();
    store
        .connection
        .execute(
            "UPDATE network_identity SET network_id='baf5c8095f41b92ea539a10456da64e3',dns=?1",
            [r#"{"enabled":true,"zone":"meshway.test","ttl_seconds":30}"#],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE configuration SET revision=42,policy=?1,source_jsonc=?1",
            [r#"{"default_action":"direct","rules":[]}"#],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_identities VALUES (?1,?2,'[]')",
            params![node.id, "09".to_owned() + &"00".repeat(31)],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_receipts VALUES (?1,42,?2,'2026-10-01T00:00:00Z')",
            params![node.id, "a".repeat(64)],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_runtime_leases VALUES (?1,?2,7)",
            params![node.id, "b".repeat(64)],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO connection_runtime_reports VALUES (?1,3,?2,'2026-10-01T00:00:00Z')",
            params![node.id, b"opaque runtime evidence".as_slice()],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO tag_definitions VALUES ('tag:existing','preserved')",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO node_tags VALUES (?1,'tag:existing')",
            [&node.id],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE mesh_access_source SET policy=?1,source_jsonc=?1",
            [r#"{"default_action":"allow","rules":[]}"#],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE nodes SET acknowledged_revision=42,acknowledged_digest=?1 WHERE id=?2",
            params!["c".repeat(64), node.id],
        )
        .unwrap();
    let old_token = store.create_token(3600).unwrap();
    let authorization = AuthorizeDeviceRequest {
        redirect_uri: DEVICE_CALLBACK_URI.into(),
        code_challenge: s256(&"v".repeat(43)),
        name: "App".into(),
        agent_version: "test".into(),
    };
    let code = store.authorize_device(&authorization).unwrap();
    let before = rows(&store.connection, false);
    let config = canonical_json(&store.configuration(&first.node_credential).unwrap()).unwrap();
    let dns = canonical_json(&store.agent_dns(&first.node_credential).unwrap()).unwrap();
    // Force a migration failure after table creation/copy has begun. FK check
    // detects the corruption; every DDL operation and user_version rolls back.
    store
        .connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO node_tags VALUES ('missing','tag:existing')",
            [],
        )
        .unwrap();
    store
        .connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let invalid = rows(&store.connection, false);
    assert!(upgrade(&mut store.connection).is_err());
    assert_eq!(rows(&store.connection, false), invalid);
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        8
    );
    assert!(
        store
            .connection
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    store
        .connection
        .execute("DELETE FROM node_tags WHERE node_id='missing'", [])
        .unwrap();
    drop(store);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(rows(&store.connection, true), before);
    assert_eq!(
        store.network().unwrap().network_id,
        "baf5c8095f41b92ea539a10456da64e3"
    );
    assert_eq!(store.revision().unwrap(), 42);
    assert_eq!(
        canonical_json(&store.configuration(&first.node_credential).unwrap()).unwrap(),
        config
    );
    assert_eq!(
        canonical_json(&store.agent_dns(&first.node_credential).unwrap()).unwrap(),
        dns
    );
    assert!(matches!(
        store.heartbeat(&second.node_credential, "test"),
        Err(Error::Forbidden)
    ));
    let exchanged = store
        .exchange_device(
            &code.code,
            &ExchangeDeviceRequest {
                code_verifier: "v".repeat(43),
                redirect_uri: DEVICE_CALLBACK_URI.into(),
                node_credential: local::credential().unwrap(),
                name: "App".into(),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    assert_eq!(exchanged.mesh_ipv4, "100.96.0.3");
    assert!(
        store
            .enroll(&old_token.token, &request("legacy token"))
            .is_ok()
    );
    drop(store);
    assert!(Store::open(dir.path(), &NetworkOptions::default()).is_ok());
}

#[test]
fn networks_isolate_inventory_addresses_revision_tags_and_agent_snapshots() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let original = store.network().unwrap();
    let first = request("one");
    let one = enroll(&mut store, &first);
    let first_revision = store.revision().unwrap();
    let second_network = store
        .create_network(&CreateNetworkRequest {
            name: "Second".into(),
        })
        .unwrap();
    assert_eq!(second_network.revision, 1);
    store.select_network(&second_network.network_id).unwrap();
    assert!(store.nodes().unwrap().is_empty());
    assert!(store.tags().unwrap().tags.is_empty());
    assert!(
        store
            .connections()
            .unwrap()
            .configuration
            .listeners
            .is_empty()
    );
    assert!(store.connection_runtimes().unwrap().nodes.is_empty());
    let second = request("one");
    let two = enroll(&mut store, &second);
    assert_eq!(one.mesh_ipv4, two.mesh_ipv4);
    assert!(matches!(store.revoke(&one.id), Err(Error::NotFound)));
    assert!(matches!(
        store.set_country(&SetNodeCountryRequest {
            expected_revision: 2,
            node_id: one.id.clone(),
            country: Some("US".into())
        }),
        Err(Error::NotFound)
    ));
    assert!(
        store
            .save_mesh_access(&SaveMeshAccessRequest {
                expected_revision: 2,
                policy: MeshAccessPolicy {
                    default_action: AccessAction::Reject,
                    rules: vec![MeshAccessRule {
                        id: "cross".into(),
                        source: vec![format!("node:{}", one.id)],
                        destination: vec!["*".into()],
                        action: AccessAction::Allow,
                        bidirectional: false
                    }]
                },
                source_jsonc: None
            })
            .is_err()
    );
    assert!(
        store
            .set_node_tags(&SetNodeTagsRequest {
                expected_revision: 2,
                node_id: one.id.clone(),
                tags: vec![]
            })
            .is_err()
    );
    assert!(
        store
            .enroll(&store.create_token(3600).unwrap().token, &first)
            .is_err()
    );
    let config = store.configuration(&first.node_credential).unwrap();
    assert_eq!(config.payload.node.id, one.id);
    assert!(config.payload.peers.is_empty());
    assert_eq!(config.payload.revision, first_revision);
    assert_eq!(
        store
            .agent_dns(&first.node_credential)
            .unwrap()
            .payload
            .network_id,
        original.network_id
    );
    let config = store.configuration(&second.node_credential).unwrap();
    assert_eq!(config.payload.node.id, two.id);
    assert!(config.payload.peers.is_empty());
    assert_eq!(
        store
            .agent_dataplane(&second.node_credential)
            .unwrap()
            .payload
            .node_id,
        two.id
    );
    assert_eq!(store.networks().unwrap().networks.len(), 2);
    store.select_network(&original.network_id).unwrap();
    assert_eq!(store.nodes().unwrap().len(), 1);
    assert_eq!(store.revision().unwrap(), first_revision);
}

#[test]
fn live_discovery_and_runtime_leases_survive_migration_and_network_switches() {
    let dir = directory();
    let mut store = Store::open_v8(dir.path(), &NetworkOptions::default()).unwrap();
    let enrollment = request("reporting");
    let node = enroll(&mut store, &enrollment);
    store
        .publish_connection_identity(
            &enrollment.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: "09".to_owned() + &"00".repeat(31),
                capabilities: vec![],
            },
        )
        .unwrap();
    store
        .begin_connection_runtime(
            &enrollment.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: local::credential().unwrap(),
                expected_epoch: 0,
            },
        )
        .unwrap();
    let snapshot = store
        .agent_connections(&enrollment.node_credential)
        .unwrap();
    store
        .report_connection_runtime(
            &enrollment.node_credential,
            &ReportConnectionRuntimeRequest {
                epoch: 1,
                sequence: 1,
                revision: snapshot.payload.revision,
                digest: snapshot.digest.clone(),
                state: ConnectionRuntimeState::Disabled,
                reason: ConnectionRuntimeReason::Disabled,
                listeners: vec![],
                peers: vec![],
            },
        )
        .unwrap();
    store
        .report_endpoint_discovery(
            &enrollment.node_credential,
            &ReportEndpointDiscoveryRequest {
                epoch: 1,
                sequence: 1,
                revision: snapshot.payload.revision,
                digest: snapshot.digest,
                state: EndpointDiscoveryState::Disabled,
                sockets: vec![],
            },
        )
        .unwrap();
    let runtime = canonical_json(&store.connection_runtimes().unwrap()).unwrap();
    let discovery = canonical_json(&store.endpoint_discoveries().unwrap()).unwrap();
    upgrade(&mut store.connection).unwrap();
    install_scope(&store.connection).unwrap();
    assert_eq!(
        canonical_json(&store.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&store.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
    let original = store.network().unwrap();
    let second = store
        .create_network(&CreateNetworkRequest {
            name: "isolated".into(),
        })
        .unwrap();
    store.select_network(&second.network_id).unwrap();
    assert!(store.connection_runtimes().unwrap().nodes.is_empty());
    assert!(store.endpoint_discoveries().unwrap().nodes.is_empty());
    let other = request("other");
    let other_node = enroll(&mut store, &other);
    store
        .save_tags(&SaveTagsRequest {
            expected_revision: store.revision().unwrap(),
            tags: vec![TagDefinition {
                tag: "tag:local".into(),
                description: "only second network".into(),
            }],
        })
        .unwrap();
    // Composite FKs enforce network ownership even for physical SQL writes.
    assert!(store.connection.execute("INSERT INTO main.node_tags_scoped(network_id,node_id,tag) VALUES (?1,?2,'tag:local')", params![second.network_id,node.id]).is_err());
    assert_eq!(
        store.endpoint_discoveries().unwrap().nodes[0].node_id,
        other_node.id
    );
    assert_eq!(store.endpoint_discoveries().unwrap().nodes[0].sequence, 0);
    assert_eq!(
        store
            .get_connection_runtime_epoch(&enrollment.node_credential)
            .unwrap()
            .epoch,
        1
    );
    store.select_network(&original.network_id).unwrap();
    assert_eq!(
        canonical_json(&store.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&store.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
}
