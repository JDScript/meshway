use super::{TABLES, upgrade};
#[path = "original_v8.rs"]
mod original_v8;
use crate::{
    Error, NetworkOptions,
    store::{
        NetworkId, Store,
        test_store::TestStore,
        tests::{directory, request},
    },
};
use meshway_protocol::*;
use rusqlite::{Connection, params, types::Value};

fn rows(db: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    TABLES
        .iter()
        .map(|(table, columns)| {
            let mut query = db
                .prepare(&format!("SELECT {columns} FROM {table} ORDER BY 1,2"))
                .unwrap();
            let count = query.column_count();
            let values = query
                .query_map([], |row| {
                    (0..count)
                        .map(|i| row.get(i))
                        .collect::<Result<Vec<Value>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            ((*table).to_owned(), values)
        })
        .collect()
}
fn enrolled(store: &mut Store, network: &NetworkId, request: &EnrollAgentRequest) -> NodeSummary {
    let token = store.create_token(network, 3600).unwrap();
    store.enroll(&token.token, request).unwrap()
}

#[test]
fn complete_v8_upgrade_is_lossless_and_rolls_back_on_invalid_ownership() {
    let dir = directory();
    let mut fixture = TestStore::open(
        dir.path(),
        &NetworkOptions {
            ipv4_prefix: Some("100.96.0.0/16".into()),
            ipv6_prefix: Some("fd73:96a6:cf5e::/64".into()),
        },
    )
    .unwrap();
    fixture.set_fixture_network_id("baf5c8095f41b92ea539a10456da64e3");
    let network = fixture.network_id.clone();
    let first = request("existing");
    let node = enrolled(&mut fixture, &network, &first);
    let second = request("revoked");
    let revoked = enrolled(&mut fixture, &network, &second);
    fixture.revoke(&revoked.id).unwrap();
    fixture
        .connection
        .execute(
            "UPDATE network_identity SET dns=?1 WHERE network_id=?2",
            params![
                r#"{"enabled":true,"zone":"meshway.test","ttl_seconds":30}"#,
                network.as_str()
            ],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE configuration SET revision=42,policy=?1,source_jsonc=?1 WHERE network_id=?2",
            params![
                r#"{"default_action":"direct","rules":[]}"#,
                network.as_str()
            ],
        )
        .unwrap();
    fixture.connection.execute("INSERT INTO connection_identities(node_id,public_key,capabilities,network_id) VALUES(?1,?2,'[]',?3)",params![node.id,"09".to_owned()+&"00".repeat(31),network.as_str()]).unwrap();
    fixture.connection.execute("INSERT INTO connection_receipts(node_id,revision,digest,saved_at,network_id) VALUES(?1,42,?2,'2026-10-01T00:00:00Z',?3)",params![node.id,"a".repeat(64),network.as_str()]).unwrap();
    fixture.connection.execute("INSERT INTO connection_runtime_leases(node_id,instance_id,epoch,network_id) VALUES(?1,?2,7,?3)",params![node.id,"b".repeat(64),network.as_str()]).unwrap();
    fixture.connection.execute("INSERT INTO connection_runtime_reports(node_id,sequence,body,reported_at,network_id) VALUES(?1,3,?2,'2026-10-01T00:00:00Z',?3)",params![node.id,b"opaque runtime evidence".as_slice(),network.as_str()]).unwrap();
    fixture.connection.execute("INSERT INTO tag_definitions(tag,description,network_id) VALUES('tag:existing','preserved',?1)",[network.as_str()]).unwrap();
    fixture
        .connection
        .execute(
            "INSERT INTO node_tags(node_id,tag,network_id) VALUES(?1,'tag:existing',?2)",
            params![node.id, network.as_str()],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE mesh_access_source SET policy=?1,source_jsonc=?1 WHERE network_id=?2",
            params![r#"{"default_action":"allow","rules":[]}"#, network.as_str()],
        )
        .unwrap();
    fixture.connection.execute("UPDATE connection_source SET configuration=?1 WHERE network_id=?2",params![serde_json::json!({"listeners":[{"id":"preserved","node_id":node.id,"transport":"native_udp","bind":"0.0.0.0:42000","advertised_endpoints":[],"credential_ref":null}],"candidates":[]}).to_string(),network.as_str()]).unwrap();
    fixture.connection.execute("UPDATE nodes SET acknowledged_revision=42,acknowledged_digest=?1 WHERE id=?2 AND network_id=?3",params!["c".repeat(64),node.id,network.as_str()]).unwrap();
    let token = fixture.create_token(3600).unwrap();
    let code = fixture
        .authorize_device(&AuthorizeDeviceRequest {
            redirect_uri: DEVICE_CALLBACK_URI.into(),
            code_challenge: s256(&"v".repeat(43)),
            name: "App".into(),
            agent_version: "test".into(),
        })
        .unwrap();
    let config = canonical_json(&fixture.configuration(&first.node_credential).unwrap()).unwrap();
    let dns = canonical_json(&fixture.agent_dns(&first.node_credential).unwrap()).unwrap();
    fixture.downgrade_v8();
    let before = rows(&fixture.connection);
    fixture
        .connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    fixture
        .connection
        .execute("INSERT INTO node_tags VALUES('missing','tag:existing')", [])
        .unwrap();
    fixture
        .connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let invalid = rows(&fixture.connection);
    assert!(upgrade(&mut fixture.connection).is_err());
    assert_eq!(rows(&fixture.connection), invalid);
    assert_eq!(
        fixture
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        8
    );
    assert!(
        fixture
            .connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
            .unwrap()
    );
    fixture
        .connection
        .execute("DELETE FROM node_tags WHERE node_id='missing'", [])
        .unwrap();
    drop(fixture);
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let network = store
        .network_id("baf5c8095f41b92ea539a10456da64e3")
        .unwrap();
    assert_eq!(rows(&store.connection), before);
    assert_eq!(store.revision(&network).unwrap(), 42);
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
    assert!(
        store
            .exchange_device(
                &code.code,
                &ExchangeDeviceRequest {
                    code_verifier: "v".repeat(43),
                    redirect_uri: DEVICE_CALLBACK_URI.into(),
                    node_credential: local::credential().unwrap(),
                    name: "App".into(),
                    agent_version: "test".into()
                }
            )
            .is_ok()
    );
    assert!(store.enroll(&token.token, &request("legacy token")).is_ok());
    assert!(
        store
            .connection
            .prepare("SELECT name FROM sqlite_temp_master WHERE type IN ('view','trigger','table')")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    drop(store);
    assert!(Store::open(dir.path(), &NetworkOptions::default()).is_ok());
}

#[test]
fn networks_isolate_inventory_addresses_revision_tags_and_agent_snapshots() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let first_network = store.initial_network_id().unwrap();
    let first = request("one");
    let one = enrolled(&mut store, &first_network, &first);
    let first_revision = store.revision(&first_network).unwrap();
    let second_snapshot = store
        .create_network(&CreateNetworkRequest {
            name: "Second".into(),
        })
        .unwrap();
    let second_network = store.network_id(&second_snapshot.network_id).unwrap();
    assert_eq!(second_snapshot.revision, 1);
    assert!(store.nodes(&second_network).unwrap().is_empty());
    assert!(store.tags(&second_network).unwrap().tags.is_empty());
    assert!(
        store
            .connections(&second_network)
            .unwrap()
            .configuration
            .listeners
            .is_empty()
    );
    assert!(
        store
            .connection_runtimes(&second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    let second = request("one");
    let two = enrolled(&mut store, &second_network, &second);
    assert_eq!(one.mesh_ipv4, two.mesh_ipv4);
    assert!(matches!(
        store.revoke(&second_network, &one.id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.set_country(
            &second_network,
            &SetNodeCountryRequest {
                expected_revision: 2,
                node_id: one.id.clone(),
                country: Some("US".into())
            }
        ),
        Err(Error::NotFound)
    ));
    assert!(
        store
            .save_mesh_access(
                &second_network,
                &SaveMeshAccessRequest {
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
                }
            )
            .is_err()
    );
    assert!(
        store
            .set_node_tags(
                &second_network,
                &SetNodeTagsRequest {
                    expected_revision: 2,
                    node_id: one.id.clone(),
                    tags: vec![]
                }
            )
            .is_err()
    );
    let token = store.create_token(&second_network, 3600).unwrap();
    assert!(store.enroll(&token.token, &first).is_err());
    for _ in 0..16 {
        // Immutable IDs can be retained and used in any order. Global listing,
        // network creation and agent requests never mutate their meaning.
        assert_eq!(store.nodes(&first_network).unwrap()[0].id, one.id);
        assert_eq!(store.nodes(&second_network).unwrap()[0].id, two.id);
        assert!(
            store
                .configuration(&first.node_credential)
                .unwrap()
                .payload
                .peers
                .is_empty()
        );
        assert!(
            store
                .configuration(&second.node_credential)
                .unwrap()
                .payload
                .peers
                .is_empty()
        );
        assert_eq!(store.networks().unwrap().networks.len(), 2);
        assert_eq!(store.revision(&first_network).unwrap(), first_revision);
        assert_eq!(
            store
                .agent_dns(&first.node_credential)
                .unwrap()
                .payload
                .network_id,
            first_network.as_str()
        );
    }
    assert_eq!(
        store
            .agent_dataplane(&second.node_credential)
            .unwrap()
            .payload
            .node_id,
        two.id
    );
}

#[test]
fn live_discovery_and_runtime_leases_survive_migration_and_network_switches() {
    let dir = directory();
    let mut fixture = TestStore::open(dir.path(), &NetworkOptions::default()).unwrap();
    let enrollment = request("reporting");
    let network = fixture.network_id.clone();
    let node = enrolled(&mut fixture, &network, &enrollment);
    fixture
        .publish_connection_identity(
            &enrollment.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: "09".to_owned() + &"00".repeat(31),
                capabilities: vec![],
            },
        )
        .unwrap();
    fixture
        .begin_connection_runtime(
            &enrollment.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: local::credential().unwrap(),
                expected_epoch: 0,
            },
        )
        .unwrap();
    let snapshot = fixture
        .agent_connections(&enrollment.node_credential)
        .unwrap();
    fixture
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
    fixture
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
    let runtime = canonical_json(&fixture.connection_runtimes().unwrap()).unwrap();
    let discovery = canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap();
    fixture.downgrade_v8();
    upgrade(&mut fixture.connection).unwrap();
    assert_eq!(
        canonical_json(&fixture.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
    let second = fixture
        .create_network(&CreateNetworkRequest {
            name: "isolated".into(),
        })
        .unwrap();
    let second_network = fixture.network_id(&second.network_id).unwrap();
    assert!(
        Store::connection_runtimes(&fixture, &second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    assert!(
        Store::endpoint_discoveries(&fixture, &second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    let other = enrolled(&mut fixture, &second_network, &request("other"));
    Store::save_tags(
        &mut fixture,
        &second_network,
        &SaveTagsRequest {
            expected_revision: 2,
            tags: vec![TagDefinition {
                tag: "tag:local".into(),
                description: "second only".into(),
            }],
        },
    )
    .unwrap();
    assert!(
        fixture
            .connection
            .execute(
                "INSERT INTO node_tags(network_id,node_id,tag) VALUES(?1,?2,'tag:local')",
                params![second_network.as_str(), node.id]
            )
            .is_err()
    );
    assert_eq!(
        Store::endpoint_discoveries(&fixture, &second_network)
            .unwrap()
            .nodes[0]
            .node_id,
        other.id
    );
    assert_eq!(
        fixture
            .get_connection_runtime_epoch(&enrollment.node_credential)
            .unwrap()
            .epoch,
        1
    );
    assert_eq!(
        canonical_json(&fixture.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
}
