use crate::{NetworkOptions, store::Store};
use meshway_protocol::EnrollAgentRequest;
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
fn standalone_v10_fragment_preserves_every_legacy_cell_and_identity_without_version_assignment() {
    let historical = crate::store::test_support::fixtures::original_v8::V8::create();
    historical.write_scoped_schema(10);
    // Independent old DDL/rows, without opening or downgrading a current Store.
    let mut db = historical.database();
    let node_id = &historical.ids[0];
    assert!(
        !db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let before = snapshot(&db);
    let version_before = db
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    super::install(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(snapshot(&db), before);
    assert!(db.execute("DELETE FROM node_ownership", []).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        version_before
    );
    assert_eq!(
        db.query_row(
            "SELECT membership_id FROM node_ownership WHERE node_id=?1",
            [node_id],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM account_users", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    let mut store = Store::open(historical.directory.path(), &NetworkOptions::default()).unwrap();
    assert!(
        store
            .heartbeat(&historical.credentials[0], "after migration")
            .is_ok()
    );
    let request = EnrollAgentRequest {
        node_credential: historical.credentials[0].clone(),
        name: "alpha".into(),
        agent_version: "v8".into(),
    };
    assert_eq!(
        store.enroll(&historical.bound_token, &request).unwrap().id,
        *node_id
    );
}

#[test]
fn fragment_failure_rolls_back_all_ddl_and_preserves_v9() {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(include_str!("../../test_support/fixtures/schema_v9.sql"))
        .unwrap();
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

fn structural_fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(include_str!("../../test_support/fixtures/schema_v9.sql"))
        .unwrap();
    db.execute_batch(super::SCHEMA).unwrap();
    db.execute_batch(include_str!("../sql/ownership.sql"))
        .unwrap();
    super::validate(&db).unwrap();
    db
}

#[test]
fn installed_gate_rejects_missing_or_same_name_weakened_guards() {
    for guard in [
        "ownership_new_node",
        "ownership_immutable",
        "ownership_no_delete",
        "device_account_binding_immutable",
        "device_account_binding_no_delete",
        "account_user_identity",
        "account_membership_identity",
        "account_invitation_binding",
        "account_reset_binding",
    ] {
        let db = structural_fixture();
        let definition: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1",
                [guard],
                |r| r.get(0),
            )
            .unwrap();
        db.execute_batch(&format!("DROP TRIGGER {guard}")).unwrap();
        assert!(super::validate(&db).is_err(), "missing {guard}");
        let offset = definition.find("BEGIN").unwrap();
        db.execute_batch(&format!("{} BEGIN SELECT 1; END", &definition[..offset]))
            .unwrap();
        assert!(super::validate(&db).is_err(), "weakened {guard}");
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
    }
}

#[test]
fn installed_gate_checks_partial_unique_index_and_composite_foreign_keys() {
    for index in [
        "account_active_membership",
        "account_membership_subject",
        "account_enrollment_scope",
    ] {
        let db = structural_fixture();
        let definition: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1",
                [index],
                |r| r.get(0),
            )
            .unwrap();
        db.execute_batch(&format!("DROP INDEX {index}")).unwrap();
        assert!(super::validate(&db).is_err());
        db.execute_batch(&definition.replacen("CREATE UNIQUE INDEX", "CREATE INDEX", 1))
            .unwrap();
        assert!(super::validate(&db).is_err(), "nonunique {index}");
    }
    let db = structural_fixture();
    db.execute_batch("DROP INDEX account_active_membership; CREATE UNIQUE INDEX account_active_membership ON account_memberships(network_id,user_id) WHERE revoked_at IS NOT NULL;").unwrap();
    assert!(super::validate(&db).is_err());
    for (from, to) in [
        (
            "FOREIGN KEY(network_id,membership_id) REFERENCES account_memberships(network_id,membership_id)",
            "FOREIGN KEY(membership_id) REFERENCES account_memberships(membership_id)",
        ),
        (
            "FOREIGN KEY(network_id,membership_id,user_id) REFERENCES account_memberships(network_id,membership_id,user_id)",
            "FOREIGN KEY(membership_id) REFERENCES account_memberships(membership_id)",
        ),
        (
            "FOREIGN KEY(network_id,node_id) REFERENCES nodes(network_id,id)",
            "FOREIGN KEY(node_id) REFERENCES nodes(id)",
        ),
    ] {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db.execute_batch(include_str!("../../test_support/fixtures/schema_v9.sql"))
            .unwrap();
        db.execute_batch(super::SCHEMA).unwrap();
        let ddl = include_str!("../sql/ownership.sql");
        assert!(ddl.contains(from));
        db.execute_batch(&ddl.replace(from, to)).unwrap();
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        assert!(super::validate(&db).is_err(), "weakened composite FK");
    }
}

#[test]
fn installed_gate_rejects_partial_tables_and_nodes_without_same_network_ownership() {
    let db = structural_fixture();
    db.execute_batch("DROP TABLE account_password_resets")
        .unwrap();
    assert!(super::present(&db).unwrap());
    assert!(super::validate(&db).is_err());
    let db = structural_fixture();
    db.execute_batch(
        "INSERT INTO networks VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','fixture',1);",
    )
    .unwrap();
    let insert = "INSERT INTO nodes(id,network_id,credential_hash,name,agent_version,registered_at,last_seen,mesh_ipv4,mesh_ipv6) VALUES('node','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',zeroblob(32),'fixture','fixture','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z','100.96.0.1','fd00::1')";
    let guard: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='ownership_new_node'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER ownership_new_node").unwrap();
    db.execute_batch(insert).unwrap();
    db.execute_batch(&guard).unwrap();
    assert!(
        !db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    assert!(
        super::validate(&db).is_err(),
        "guard restored but missing row still rejected"
    );
    db.execute_batch("INSERT INTO node_ownership(network_id,node_id) VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','node')").unwrap();
    super::validate(&db).unwrap();
    assert!(db.execute_batch("INSERT INTO node_ownership(network_id,node_id) VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','node')").is_err());
    db.pragma_update(None, "foreign_keys", false).unwrap();
    assert!(super::validate(&db).is_err());
}
