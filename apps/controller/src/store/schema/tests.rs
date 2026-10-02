use super::*;
use crate::store::{Store, test_support::directory};

#[test]
fn complete_schema_installation_rolls_back_as_one_transaction() {
    let mut db = Connection::open_in_memory().unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch(SQL).unwrap();
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn restart_preserves_policy_and_rejects_unknown_or_damaged_schema_without_repair() {
    for damage in [
        "PRAGMA user_version=12",
        "DROP TRIGGER ownership_new_node",
        "ALTER TABLE network_policy ADD COLUMN unknown TEXT",
        "DELETE FROM network_policy",
    ] {
        let dir = directory();
        let store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
        let scope = store.initial_network_id().unwrap();
        assert_eq!(
            store.network_policy(&scope).unwrap().policy,
            meshway_policy::NetworkPolicy::default_template()
        );
        drop(store);
        let store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
        store.connection.execute_batch(damage).unwrap();
        let before = objects(&store.connection).unwrap();
        let admin = std::fs::read(dir.path().join("admin.token")).unwrap();
        drop(store);
        assert!(Store::open(dir.path(), &NetworkOptions::default()).is_err());
        let db = Connection::open(dir.path().join("controller.sqlite3")).unwrap();
        assert_eq!(objects(&db).unwrap(), before);
        assert_eq!(
            std::fs::read(dir.path().join("admin.token")).unwrap(),
            admin
        );
    }
}

#[test]
fn current_identity_guards_forbid_node_and_enrollment_rebinding() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let scope = store.initial_network_id().unwrap();
    let token = store.create_token(&scope, 3600).unwrap();
    let request = crate::store::test_support::request("identity");
    let node = store.enroll(&token.token, &request).unwrap();
    for sql in [
        "UPDATE nodes SET credential_hash=zeroblob(32)",
        "UPDATE nodes SET network_id='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'",
        "UPDATE enrollment_tokens SET used_node_id=NULL,binding_hash=NULL",
    ] {
        assert!(store.connection.execute_batch(sql).is_err());
    }
    assert_eq!(store.enroll(&token.token, &request).unwrap().id, node.id);
}
