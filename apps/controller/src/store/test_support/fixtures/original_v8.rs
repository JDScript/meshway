//! Independent frozen v8 fixture: direct old DDL and rows, never Store::open,
//! v9 schema, a downgrade, or a v9 business mutation. Reference projections
//! use the unchanged v1 wire types and pure v2 compiler/projection functions.
use crate::store::migrations::networks::{TABLES, upgrade};
use crate::{Error, NetworkOptions, store::Store};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, params, types::Value};

pub(crate) const NETWORK: &str = "baf5c8095f41b92ea539a10456da64e3";
const STAMP: &str = "2026-10-01T00:00:00Z";

pub(crate) struct V8 {
    pub directory: tempfile::TempDir,
    pub admin: String,
    pub ids: Vec<String>,
    pub credentials: Vec<String>,
    pub unused_token: String,
    pub bound_token: String,
    pub device_code: String,
    pub exchange: ExchangeDeviceRequest,
    routing_jsonc: String,
    access_jsonc: String,
}

impl V8 {
    pub(crate) fn create() -> Self {
        let directory = crate::store::test_support::directory();
        let admin = "aa".repeat(32);
        local::atomic_write(
            &directory.path().join("admin.token"),
            admin.as_bytes(),
            false,
        )
        .unwrap();
        local::atomic_write(&directory.path().join("controller.sqlite3"), &[], false).unwrap();
        let db = Connection::open(directory.path().join("controller.sqlite3")).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db.execute_batch(
            "CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),admin_hash BLOB NOT NULL);",
        )
        .unwrap();
        db.execute_batch(include_str!("schema_v8.sql")).unwrap();
        db.execute("INSERT INTO settings VALUES(1,?1)", [local::digest(&admin)])
            .unwrap();
        let ids: Vec<_> = (1..=4).map(|n| format!("{n:064x}")).collect();
        let credentials: Vec<_> = ["11", "22", "33", "44"]
            .into_iter()
            .map(|s| s.repeat(32))
            .collect();
        for (index, name) in ["alpha", "peer", "app", "revoked"].into_iter().enumerate() {
            db.execute("INSERT INTO nodes(id,credential_hash,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,acknowledged_revision) VALUES(?1,?2,?3,'v8',?4,?4,?5,?6,?7,?8,41)",params![ids[index],local::digest(&credentials[index]),name,STAMP,(index==3).then_some(STAMP),format!("100.96.0.{}",index+1),format!("fd73:96a6:cf5e::{}",index+1),(index==0).then_some("US")]).unwrap();
            let capabilities = vec![ConnectionCapability {
                transport: ConnectionTransport::NativeUdp,
                semantics: ConnectionSemantics::Datagram,
                can_dial: true,
                can_accept: true,
            }];
            db.execute(
                "INSERT INTO connection_identities VALUES(?1,?2,?3)",
                params![
                    ids[index],
                    format!("{:02x}{}", index + 9, "00".repeat(31)),
                    serde_json::to_string(&capabilities).unwrap()
                ],
            )
            .unwrap();
        }
        let routing = RoutingPolicy {
            default_action: DefaultRouteAction::Direct,
            rules: vec![RouteRule {
                id: "v8-direct-peer".into(),
                source: SourceSelector {
                    node_ids: vec![ids[0].clone()],
                    countries: vec![],
                },
                destination_scope: DestinationScope::Mesh,
                destination_any: vec![DestinationSelector {
                    kind: DestinationSelectorKind::Node,
                    value: ids[1].clone(),
                }],
                action: RouteAction {
                    kind: RouteActionKind::Route,
                    via: vec![],
                    exit_node_id: None,
                },
            }],
        };
        let access = MeshAccessPolicy {
            default_action: AccessAction::Reject,
            rules: vec![MeshAccessRule {
                id: "v8-tagged-pair".into(),
                source: vec![format!("node:{}", ids[0])],
                destination: vec!["tag:peer".into()],
                action: AccessAction::Allow,
                bidirectional: true,
            }],
        };
        let routing_jsonc = format!(
            "// original v8 routing: keep CRLF and comments\r\n{}\r\n",
            serde_json::to_string_pretty(&routing).unwrap()
        );
        let access_jsonc = format!(
            "/* original v8 access */\r\n{}\r\n",
            serde_json::to_string_pretty(&access).unwrap()
        );
        db.execute(
            "INSERT INTO configuration VALUES(1,42,'100.96.0.0/16','fd73:96a6:cf5e::/64',?1,?2)",
            params![serde_json::to_string(&routing).unwrap(), routing_jsonc],
        )
        .unwrap();
        db.execute(
            "INSERT INTO mesh_access_source VALUES(1,?1,?2)",
            params![serde_json::to_string(&access).unwrap(), access_jsonc],
        )
        .unwrap();
        db.execute(
            "INSERT INTO mesh_access_legacy_v5 VALUES(1,?1)",
            [r#"{"grants":[{"id":"historical","path":["old","evidence"]}]}"#],
        )
        .unwrap();
        db.execute(
            "INSERT INTO network_identity VALUES(1,?1,'Meshway',?2)",
            params![
                NETWORK,
                r#"{"enabled":true,"zone":"meshway.test","ttl_seconds":30}"#
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO tag_definitions VALUES('tag:peer','original v8 tag')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO node_tags VALUES(?1,'tag:peer')", [&ids[1]])
            .unwrap();
        let plan = ConnectionPlan {
            listeners: vec![ConnectionListener {
                id: "udp-peer".into(),
                node_id: ids[1].clone(),
                transport: ConnectionTransport::NativeUdp,
                bind: "0.0.0.0:42000".into(),
                advertised_endpoints: vec![ConnectionEndpoint {
                    host: "192.0.2.2".into(),
                    port: 42000,
                }],
                credential_ref: None,
            }],
            candidates: vec![ConnectionCandidate {
                id: "alpha-peer".into(),
                dialer_node_id: ids[0].clone(),
                listener_id: "udp-peer".into(),
                endpoint: ConnectionEndpoint {
                    host: "192.0.2.2".into(),
                    port: 42000,
                },
                priority: 0,
                credential_ref: None,
            }],
        };
        db.execute(
            "INSERT INTO connection_source VALUES(1,?1)",
            [serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        let bound_token = "66".repeat(32);
        let unused_token = "88".repeat(32);
        let device_code = "77".repeat(32);
        let enrollment = EnrollAgentRequest {
            node_credential: credentials[0].clone(),
            name: "alpha".into(),
            agent_version: "v8".into(),
        };
        db.execute(
            "INSERT INTO enrollment_tokens VALUES(?1,?2,0,?3,?4,NULL)",
            params![
                "bound-classic",
                local::digest(&bound_token),
                ids[0],
                local::digest(&serde_json::to_string(&enrollment).unwrap())
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO enrollment_tokens VALUES(?1,?2,?3,NULL,NULL,NULL)",
            params![
                "unused-classic",
                local::digest(&unused_token),
                (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp_millis()
            ],
        )
        .unwrap();
        let exchange = ExchangeDeviceRequest {
            code_verifier: "v".repeat(43),
            redirect_uri: DEVICE_CALLBACK_URI.into(),
            node_credential: credentials[2].clone(),
            name: "app".into(),
            agent_version: "v8".into(),
        };
        let authorization = AuthorizeDeviceRequest {
            redirect_uri: exchange.redirect_uri.clone(),
            code_challenge: s256(&exchange.code_verifier),
            name: exchange.name.clone(),
            agent_version: exchange.agent_version.clone(),
        };
        db.execute(
            "INSERT INTO enrollment_tokens VALUES(?1,?2,0,?3,?4,?5)",
            params![
                "bound-device",
                local::digest(&device_code),
                ids[2],
                local::digest(&serde_json::to_string(&exchange.enrollment().unwrap()).unwrap()),
                serde_json::to_string(&authorization).unwrap()
            ],
        )
        .unwrap();
        let fixture = Self {
            directory,
            admin,
            ids,
            credentials,
            unused_token,
            bound_token,
            device_code,
            exchange,
            routing_jsonc,
            access_jsonc,
        };
        for index in 0..3 {
            let mut previous = fixture.configuration(&db, index).payload;
            previous.revision = 41;
            db.execute(
                "UPDATE nodes SET acknowledged_digest=?1 WHERE id=?2",
                params![configuration_digest(&previous).unwrap(), fixture.ids[index]],
            )
            .unwrap();
            let mut previous = fixture.connections(&db, index).payload;
            previous.revision = 41;
            db.execute(
                "INSERT INTO connection_receipts VALUES(?1,41,?2,?3)",
                params![
                    fixture.ids[index],
                    connection_digest(&previous).unwrap(),
                    STAMP
                ],
            )
            .unwrap();
        }
        let config = fixture.connections(&db, 0);
        db.execute(
            "INSERT INTO connection_runtime_leases VALUES(?1,?2,7)",
            params![fixture.ids[0], "ee".repeat(32)],
        )
        .unwrap();
        let runtime = ReportConnectionRuntimeRequest {
            epoch: 7,
            sequence: 3,
            revision: 42,
            digest: config.digest,
            state: ConnectionRuntimeState::Applied,
            reason: ConnectionRuntimeReason::None,
            listeners: vec![],
            peers: vec![],
        };
        db.execute(
            "INSERT INTO connection_runtime_reports VALUES(?1,3,?2,?3)",
            params![fixture.ids[0], canonical_json(&runtime).unwrap(), STAMP],
        )
        .unwrap();
        db.execute_batch("PRAGMA user_version=8;").unwrap();
        // No v9 Store or schema has been involved in creating these old rows.
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        fixture
    }
    pub(crate) fn database(&self) -> Connection {
        Connection::open(self.directory.path().join("controller.sqlite3")).unwrap()
    }
    pub(crate) fn write_scoped_schema(&self, version: u32) {
        assert!(matches!(version, 9 | 10));
        let path = self.directory.path().join("historical.sqlite3");
        local::atomic_write(&path, &[], false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),admin_hash BLOB NOT NULL);",
        )
        .unwrap();
        db.execute_batch(include_str!("schema_v9.sql")).unwrap();
        if version == 10 {
            db.execute_batch(include_str!("schema_v10.sql")).unwrap();
        } else {
            db.pragma_update(None, "user_version", 9).unwrap();
        }
        db.execute(
            "INSERT INTO networks VALUES(?1,?2,1)",
            params![NETWORK, STAMP],
        )
        .unwrap();
        db.execute(
            "ATTACH DATABASE ?1 AS frozen",
            [self
                .directory
                .path()
                .join("controller.sqlite3")
                .to_str()
                .unwrap()],
        )
        .unwrap();
        db.execute_batch("INSERT INTO settings SELECT * FROM frozen.settings;")
            .unwrap();
        for (table, columns) in TABLES {
            if *table == "network_identity" {
                db.execute_batch(&format!(
                    "INSERT INTO main.{table}({columns}) SELECT {columns} FROM frozen.{table};"
                ))
                .unwrap();
            } else {
                db.execute(&format!("INSERT INTO main.{table}({columns},network_id) SELECT {columns},?1 FROM frozen.{table}"), [NETWORK]).unwrap();
            }
        }
        db.execute_batch("DETACH DATABASE frozen;").unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        drop(db);
        std::fs::rename(path, self.directory.path().join("controller.sqlite3")).unwrap();
    }
    /// Construct a fixed older schema and copy only its original row columns.
    /// The row source is the frozen v8 fixture above, never the current Store.
    pub(crate) fn write_earlier_schema(&self, version: u32) {
        assert!(matches!(version, 3 | 5 | 6 | 7));
        let path = self.directory.path().join("historical.sqlite3");
        local::atomic_write(&path, &[], false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("schema_v3.sql")).unwrap();
        if version >= 5 {
            db.execute_batch(include_str!("schema_v5_additions.sql"))
                .unwrap();
        }
        if version >= 6 {
            db.execute_batch(include_str!("schema_v6_additions.sql"))
                .unwrap();
        }
        if version >= 7 {
            db.execute_batch(include_str!("schema_v7_additions.sql"))
                .unwrap();
        }
        db.execute(
            "ATTACH DATABASE ?1 AS frozen",
            [self
                .directory
                .path()
                .join("controller.sqlite3")
                .to_str()
                .unwrap()],
        )
        .unwrap();
        let tables = db
            .prepare("SELECT name FROM main.sqlite_schema WHERE type='table'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for table in tables {
            let columns = db
                .prepare(&format!("PRAGMA main.table_info({table})"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join(",");
            let source = if version == 5 && table == "mesh_access_source" {
                "mesh_access_legacy_v5"
            } else {
                &table
            };
            db.execute_batch(&format!(
                "INSERT INTO main.{table}({columns}) SELECT {columns} FROM frozen.{source};"
            ))
            .unwrap();
        }
        db.execute_batch("DETACH DATABASE frozen;").unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        drop(db);
        std::fs::rename(path, self.directory.path().join("controller.sqlite3")).unwrap();
    }
    pub(crate) fn nodes(&self, db: &Connection) -> Vec<NodeSummary> {
        db.prepare("SELECT id,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,(SELECT revision FROM configuration WHERE id=1),acknowledged_revision FROM nodes ORDER BY id").unwrap().query_map([],crate::store::node_row).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    }
    pub(crate) fn configuration(&self, db: &Connection, index: usize) -> ConfigurationSnapshot {
        let mut nodes: Vec<_> = self
            .nodes(db)
            .into_iter()
            .filter(|n| n.revoked_at.is_none())
            .map(|n| MeshNode {
                id: n.id,
                name: n.name,
                country: n.country,
                mesh_ipv4: n.mesh_ipv4,
                mesh_ipv6: n.mesh_ipv6,
            })
            .collect();
        let network = MeshNetwork {
            ipv4_prefix: "100.96.0.0/16".into(),
            ipv6_prefix: "fd73:96a6:cf5e::/64".into(),
        };
        let json: String = db
            .query_row("SELECT policy FROM configuration WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        let policy: RoutingPolicy = serde_json::from_str(&json).unwrap();
        let report = meshway_policy::compile(&policy, &network, &nodes);
        assert!(report.is_valid());
        let position = nodes.iter().position(|n| n.id == self.ids[index]).unwrap();
        let node = nodes.remove(position);
        let payload = NodeConfiguration {
            schema_version: 1,
            revision: 42,
            network,
            node,
            peers: nodes,
            routing_policy: report.normalized_policy,
            policy_valid: true,
            diagnostics: vec![],
        };
        ConfigurationSnapshot {
            digest: configuration_digest(&payload).unwrap(),
            payload,
        }
    }
    pub(crate) fn connections(&self, db: &Connection, index: usize) -> AgentConnectionsSnapshot {
        let nodes=self.nodes(db).into_iter().map(|node| {
            let (key,capabilities):(String,String)=db.query_row("SELECT public_key,capabilities FROM connection_identities WHERE node_id=?1",[&node.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            let receipt:Option<(u32,String,String)>=db.query_row("SELECT revision,digest,saved_at FROM connection_receipts WHERE node_id=?1",[&node.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().unwrap();
            ConnectionNode {node,public_key:Some(key),capabilities:serde_json::from_str(&capabilities).unwrap(),acknowledged_revision:receipt.as_ref().map_or(0,|r|r.0),acknowledged_digest:receipt.as_ref().map(|r|r.1.clone()),acknowledged_at:receipt.map(|r|r.2.parse().unwrap())}
        }).collect();
        let json: String = db
            .query_row(
                "SELECT configuration FROM connection_source WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let source = ConnectionsSnapshot {
            revision: 42,
            configuration: serde_json::from_str(&json).unwrap(),
            valid: true,
            diagnostics: vec![],
            nodes,
        };
        crate::store::connections::projection(&source, &self.ids[index]).unwrap()
    }
    pub(crate) fn dns(&self, db: &Connection, index: usize) -> AgentDnsSnapshot {
        let nodes = self.configuration(db, index);
        let mut inventory = nodes.payload.peers;
        inventory.push(nodes.payload.node);
        let dns = MeshDnsConfiguration {
            enabled: true,
            zone: "meshway.test".into(),
            ttl_seconds: 30,
        };
        let (records, diagnostics) = compile_dns_records(&dns, &inventory).unwrap();
        let payload = NodeDnsConfiguration {
            schema_version: 1,
            revision: 42,
            node_id: self.ids[index].clone(),
            network_id: NETWORK.into(),
            dns,
            records,
            diagnostics,
        };
        AgentDnsSnapshot {
            digest: dns_digest(&payload).unwrap(),
            payload,
        }
    }
    pub(crate) fn dataplane(&self, db: &Connection, index: usize) -> AgentDataplaneSnapshot {
        let access_policy = if index == 2 {
            MeshPathAuthorization::default()
        } else {
            MeshPathAuthorization {
                grants: vec![
                    vec![self.ids[0].clone(), self.ids[1].clone()],
                    vec![self.ids[1].clone(), self.ids[0].clone()],
                ]
                .into_iter()
                .map(|path| MeshAccessGrant {
                    id: local::digest(&format!("effective:{}", path.join(":")))
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                    path,
                })
                .collect(),
            }
        };
        let identities = if index == 2 {
            vec![]
        } else {
            vec![ConnectionPeer {
                node_id: self.ids[1 - index].clone(),
                public_key: format!("{:02x}{}", 10 - index, "00".repeat(31)),
            }]
        };
        let payload = NodeDataplane {
            schema_version: 1,
            revision: 42,
            node_id: self.ids[index].clone(),
            configuration: self.configuration(db, index).payload,
            connections: self.connections(db, index).payload,
            access_policy,
            access_valid: true,
            identities,
        };
        AgentDataplaneSnapshot {
            digest: dataplane_digest(&payload).unwrap(),
            payload,
        }
    }
}

fn original_rows(db: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    let mut tables = vec![("settings", "id,admin_hash")];
    tables.extend_from_slice(TABLES);
    tables
        .into_iter()
        .map(|(table, columns)| {
            let mut statement = db
                .prepare(&format!("SELECT {columns} FROM {table} ORDER BY 1,2"))
                .unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width)
                        .map(|i| row.get(i))
                        .collect::<Result<Vec<Value>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table.into(), rows)
        })
        .collect()
}

#[test]
fn independent_original_v8_preserves_every_old_column_and_v1_projections_then_recovers_credentials()
{
    let fixture = V8::create();
    let db = fixture.database();
    let before = original_rows(&db);
    let expected_config: Vec<_> = (0..3)
        .map(|i| canonical_json(&fixture.configuration(&db, i)).unwrap())
        .collect();
    let expected_connections: Vec<_> = (0..3)
        .map(|i| canonical_json(&fixture.connections(&db, i)).unwrap())
        .collect();
    let expected_dns: Vec<_> = (0..3)
        .map(|i| canonical_json(&fixture.dns(&db, i)).unwrap())
        .collect();
    let expected_dataplane: Vec<_> = (0..3)
        .map(|i| canonical_json(&fixture.dataplane(&db, i)).unwrap())
        .collect();
    let expected_nodes = canonical_json(&NodeList {
        nodes: fixture.nodes(&db),
    })
    .unwrap();
    let legacy_nodes = fixture.nodes(&db);
    let expected_enrollment = canonical_json(&AgentNodeResponse {
        node: legacy_nodes[0].clone(),
    })
    .unwrap();
    let expected_exchange = canonical_json(&AgentNodeResponse {
        node: legacy_nodes[2].clone(),
    })
    .unwrap();
    drop(db);
    let mut store = Store::open(fixture.directory.path(), &NetworkOptions::default()).unwrap();
    let network = store.network_id(NETWORK).unwrap();
    assert!(
        original_rows(&store.connection) == before,
        "v9 changed original v8 column bytes"
    );
    assert!(
        store
            .connection
            .prepare("SELECT 1 FROM sqlite_temp_master")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert!(!store.connection.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND (name LIKE '%_scoped' OR name LIKE '%_v8')").unwrap().exists([]).unwrap());
    assert_eq!(
        canonical_json(&NodeList {
            nodes: store.nodes(&network).unwrap()
        })
        .unwrap(),
        expected_nodes
    );
    assert_eq!(store.revision(&network).unwrap(), 42);
    assert_eq!(
        store
            .policy(&network)
            .unwrap()
            .source_jsonc
            .unwrap()
            .as_bytes(),
        fixture.routing_jsonc.as_bytes()
    );
    assert_eq!(
        store
            .mesh_access(&network)
            .unwrap()
            .source_jsonc
            .unwrap()
            .as_bytes(),
        fixture.access_jsonc.as_bytes()
    );
    for i in 0..3 {
        assert_eq!(
            canonical_json(&store.configuration(&fixture.credentials[i]).unwrap()).unwrap(),
            expected_config[i]
        );
        assert_eq!(
            canonical_json(&store.agent_connections(&fixture.credentials[i]).unwrap()).unwrap(),
            expected_connections[i]
        );
        assert_eq!(
            canonical_json(&store.agent_dns(&fixture.credentials[i]).unwrap()).unwrap(),
            expected_dns[i]
        );
        assert_eq!(
            canonical_json(&store.agent_dataplane(&fixture.credentials[i]).unwrap()).unwrap(),
            expected_dataplane[i]
        );
    }
    let original = EnrollAgentRequest {
        node_credential: fixture.credentials[0].clone(),
        name: "alpha".into(),
        agent_version: "v8".into(),
    };
    assert_eq!(
        canonical_json(&AgentNodeResponse {
            node: store.enroll(&fixture.bound_token, &original).unwrap()
        })
        .unwrap(),
        expected_enrollment
    );
    assert_eq!(
        canonical_json(&AgentNodeResponse {
            node: store
                .exchange_device(&fixture.device_code, &fixture.exchange)
                .unwrap()
        })
        .unwrap(),
        expected_exchange
    );
    assert_eq!(store.revision(&network).unwrap(), 42);
    store.authorize_admin(&fixture.admin).unwrap();
    assert_eq!(
        store.heartbeat(&fixture.credentials[0], "v8").unwrap().id,
        fixture.ids[0]
    );
    assert_eq!(
        store.enroll(&fixture.bound_token, &original).unwrap().id,
        fixture.ids[0]
    );
    let fresh = EnrollAgentRequest {
        node_credential: "55".repeat(32),
        name: "fresh".into(),
        agent_version: "v8".into(),
    };
    let first = store.enroll(&fixture.unused_token, &fresh).unwrap();
    assert_eq!(first.mesh_ipv4, "100.96.0.5");
    store
        .connection
        .execute(
            "UPDATE enrollment_tokens SET expires_at=0 WHERE token_hash=?1",
            [local::digest(&fixture.unused_token)],
        )
        .unwrap();
    assert_eq!(
        store.enroll(&fixture.unused_token, &fresh).unwrap().id,
        first.id
    );
    assert_eq!(
        store
            .exchange_device(&fixture.device_code, &fixture.exchange)
            .unwrap()
            .id,
        fixture.ids[2]
    );
    assert_eq!(
        store
            .exchange_device(&fixture.device_code, &fixture.exchange)
            .unwrap()
            .id,
        fixture.ids[2]
    );
    store.revoke(&network, &fixture.ids[2]).unwrap();
    assert!(matches!(
        store.exchange_device(&fixture.device_code, &fixture.exchange),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.heartbeat(&fixture.credentials[3], "v8"),
        Err(Error::Forbidden)
    ));
}

#[test]
fn original_v8_fk_copy_and_address_validation_failures_roll_back_ddl_version_and_fk_mode() {
    for corruption in [
        "foreign-key",
        "copy-null",
        "address-pool",
        "wrong-family",
        "reserved-host",
        "duplicate-ipv6",
    ] {
        let fixture = V8::create();
        let mut db = fixture.database();
        db.pragma_update(None, "foreign_keys", false).unwrap();
        match corruption {
            "foreign-key" => {
                db.execute("INSERT INTO node_tags VALUES('missing','tag:peer')", [])
                    .unwrap();
            }
            "copy-null" => {
                db.execute(
                    "UPDATE nodes SET mesh_ipv6=NULL WHERE id=?1",
                    [&fixture.ids[3]],
                )
                .unwrap();
            }
            "address-pool" => {
                db.execute(
                    "UPDATE nodes SET mesh_ipv4='192.0.2.10' WHERE id=?1",
                    [&fixture.ids[3]],
                )
                .unwrap();
            }
            "wrong-family" => {
                db.execute(
                    "UPDATE nodes SET mesh_ipv6='100.96.0.4' WHERE id=?1",
                    [&fixture.ids[3]],
                )
                .unwrap();
            }
            "reserved-host" => {
                db.execute(
                    "UPDATE nodes SET mesh_ipv4='100.96.0.0' WHERE id=?1",
                    [&fixture.ids[3]],
                )
                .unwrap();
            }
            _ => {
                db.execute(
                    "UPDATE nodes SET mesh_ipv6='fd73:96a6:cf5e:0:0:0:0:1' WHERE id=?1",
                    [&fixture.ids[3]],
                )
                .unwrap();
            }
        }
        db.pragma_update(None, "foreign_keys", true).unwrap();
        let before = original_rows(&db);
        let schema = db
            .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(upgrade(&mut db).is_err(), "migration accepted {corruption}");
        assert!(
            original_rows(&db) == before,
            "failed migration changed original values"
        );
        let after = db
            .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(schema, after);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            8
        );
        assert!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap()
        );
    }
}

#[test]
fn startup_audits_revoked_addresses_and_identity_guards_reject_bare_sql_rebinding() {
    let fixture = V8::create();
    let mut store = Store::open(fixture.directory.path(), &NetworkOptions::default()).unwrap();
    let other = store
        .create_network(&CreateNetworkRequest {
            name: "other".into(),
        })
        .unwrap();
    for sql in [
        "UPDATE nodes SET network_id=?1 WHERE id=?2",
        "UPDATE nodes SET credential_hash=?1 WHERE id=?2",
    ] {
        assert!(
            store
                .connection
                .execute(sql, params![other.network_id, fixture.ids[1]])
                .is_err()
        );
    }
    assert!(
        store
            .connection
            .execute(
                "UPDATE connection_identities SET public_key=?1 WHERE node_id=?2",
                params!["0d".to_owned() + &"00".repeat(31), fixture.ids[1]]
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "UPDATE enrollment_tokens SET used_node_id=?1 WHERE token_hash=?2",
                params![fixture.ids[1], local::digest(&fixture.bound_token)]
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "UPDATE enrollment_tokens SET device_binding=NULL WHERE token_hash=?1",
                [local::digest(&fixture.device_code)]
            )
            .is_err()
    );
    store
        .connection
        .execute(
            "UPDATE nodes SET mesh_ipv4='192.0.2.99' WHERE id=?1",
            [&fixture.ids[3]],
        )
        .unwrap();
    drop(store);
    assert!(Store::open(fixture.directory.path(), &NetworkOptions::default()).is_err());
}
