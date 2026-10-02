use crate::{NetworkOptions, store::Store};
use meshway_protocol::{EnrollAgentRequest, local};
use rusqlite::{Connection, TransactionBehavior, types::Value};

fn snapshot(db: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    super::super::networks::TABLES
        .iter()
        .map(|(name, _)| {
            let mut q = db
                .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
                .unwrap();
            let columns = q.column_count();
            let rows = q
                .query_map([], |r| {
                    (0..columns)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name.to_string(), rows)
        })
        .collect()
}

#[test]
fn standalone_v9_fragment_preserves_every_legacy_cell_and_identity_without_version_assignment() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let scope = store.initial_network_id().unwrap();
    let token = store.create_token(&scope, 3600).unwrap();
    let request = EnrollAgentRequest {
        node_credential: local::credential().unwrap(),
        name: "legacy identity".into(),
        agent_version: "fixture".into(),
    };
    let node = store.enroll(&token.token, &request).unwrap();
    store.create_token(&scope, 3600).unwrap();
    store.connection.execute("INSERT INTO tag_definitions(network_id,tag,description) VALUES(?1,'tag:old','old tag bytes')",[scope.as_str()]).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO node_tags(network_id,node_id,tag) VALUES(?1,?2,'tag:old')",
            rusqlite::params![scope.as_str(), node.id],
        )
        .unwrap();
    store.connection.execute("INSERT INTO connection_identities(network_id,node_id,public_key,capabilities) VALUES(?1,?2,?3,'[\"quic\"]')",rusqlite::params![scope.as_str(),node.id,"11".repeat(32)]).unwrap();
    store
        .connection
        .execute(
            "UPDATE configuration SET source_jsonc=?1 WHERE network_id=?2",
            rusqlite::params![
                "// exact comment\n{ \"default_action\": \"reject\", \"rules\": [] }",
                scope.as_str()
            ],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE mesh_access_source SET source_jsonc=?1 WHERE network_id=?2",
            rusqlite::params![
                "// access comment\n{ \"default_action\": \"allow\", \"rules\": [] }",
                scope.as_str()
            ],
        )
        .unwrap();
    let before = snapshot(&store.connection);
    let version_before = store
        .connection
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .unwrap();
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    super::install(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(snapshot(&store.connection), before);
    assert!(
        store
            .connection
            .execute("DELETE FROM node_ownership", [])
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        version_before
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT membership_id FROM node_ownership WHERE node_id=?1",
                [&node.id],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM account_users", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        store
            .heartbeat(&request.node_credential, "after migration")
            .is_ok()
    );
    assert_eq!(store.enroll(&token.token, &request).unwrap().id, node.id);
}

#[test]
fn fragment_failure_rolls_back_all_ddl_and_preserves_v9() {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(include_str!("../schema_v9.sql")).unwrap();
    db.execute_batch("PRAGMA user_version=9; CREATE TABLE node_ownership(injected INTEGER);")
        .unwrap();
    {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(super::install(&tx).is_err());
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'account_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert!(
        !db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}
