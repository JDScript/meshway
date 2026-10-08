use super::*;

// Schema/account tests use a synthetic binding; filesystem and cryptographic
// binding behavior is exercised through the real App/secrets initializer.
pub(crate) fn initialize_fixture(connection: &mut Connection) -> Result<(), SchemaError> {
    super::initialize(connection, |connection, fresh| {
        if fresh {
            connection.execute("INSERT INTO master_key_binding VALUES (1, x'00000000000000000000000000000001', zeroblob(61))", [])?;
        }
        Ok(())
    })
}

use initialize_fixture as initialize;

#[test]
fn master_key_binding_is_singleton_immutable_and_required() {
    let mut connection = database();
    for mutation in [
        "INSERT INTO master_key_binding VALUES (2, randomblob(16), zeroblob(61))",
        "INSERT INTO master_key_binding VALUES (1, randomblob(16), zeroblob(61))",
        "UPDATE master_key_binding SET key_id = randomblob(16)",
        "UPDATE master_key_binding SET key_check = zeroblob(61)",
        "DELETE FROM master_key_binding",
    ] {
        assert!(connection.execute(mutation, []).is_err());
    }
    check(&mut connection).unwrap();
    connection
        .execute_batch("DROP TRIGGER master_key_binding_no_delete; DELETE FROM master_key_binding")
        .unwrap();
    connection.execute_batch("CREATE TRIGGER master_key_binding_no_delete BEFORE DELETE ON master_key_binding\nBEGIN SELECT RAISE(ABORT, 'immutable master key binding'); END").unwrap();
    assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
    assert_eq!(initialize(&mut connection), Err(SchemaError::Integrity));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM master_key_binding", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    initialize(&mut connection).unwrap();
    connection
}

#[test]
fn challenge_clock_is_persistent_monotonic_singleton_and_never_repaired() {
    let mut db = database();
    db.execute("UPDATE node_challenge_clock SET utc_ms=1234", [])
        .unwrap();
    for mutation in [
        "UPDATE node_challenge_clock SET utc_ms=1233",
        "UPDATE node_challenge_clock SET singleton=2",
        "DELETE FROM node_challenge_clock",
        "INSERT INTO node_challenge_clock VALUES(1,0)",
        "INSERT OR REPLACE INTO node_challenge_clock VALUES(1,0)",
    ] {
        assert!(db.execute(mutation, []).is_err(), "{mutation}");
    }
    initialize(&mut db).unwrap();
    assert_eq!(
        db.query_row("SELECT utc_ms FROM node_challenge_clock", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1234
    );
    let trigger: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='node_challenge_clock_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch(
        "DROP TRIGGER node_challenge_clock_no_delete; DELETE FROM node_challenge_clock",
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let before = db.total_changes();
    assert_eq!(check(&mut db), Err(SchemaError::Integrity));
    assert_eq!(initialize(&mut db), Err(SchemaError::Integrity));
    assert_eq!(db.total_changes(), before);
}

#[test]
fn ip_dataset_heads_enforce_kind_and_scope_and_missing_heads_fail_integrity() {
    let mut db = database();
    for (id, kind, format) in [
        ("city", "geoip", "mmdb"),
        ("asn", "asn", "mmdb"),
        ("rules", "ruleSet", "meshway-rules-v1"),
        ("wrong-format", "geoip", "unknown"),
    ] {
        db.execute(
            "INSERT INTO dataset_artifacts VALUES(?1,NULL,?2,'test',?3,x'01',?4,1000)",
            rusqlite::params![id, kind, format, "0".repeat(64)],
        )
        .unwrap();
    }
    for (kind, artifact) in [
        ("geoip", "asn"),
        ("geoip", "rules"),
        ("geoip", "wrong-format"),
        ("geoip", "missing"),
        ("ruleSet", "rules"),
    ] {
        assert!(
            db.execute(
                "INSERT INTO ip_dataset_heads VALUES(?1,?2)",
                [kind, artifact]
            )
            .is_err()
        );
    }
    db.execute("INSERT INTO ip_dataset_heads VALUES('geoip','city')", [])
        .unwrap();
    db.execute("INSERT INTO ip_dataset_heads VALUES('asn','asn')", [])
        .unwrap();
    assert!(
        db.execute(
            "UPDATE ip_dataset_heads SET artifact_id='asn' WHERE kind='geoip'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM ip_dataset_heads WHERE kind='geoip'", [])
            .is_err()
    );
    check(&mut db).unwrap();
    let trigger: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='ip_dataset_heads_no_delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch(
        "DROP TRIGGER ip_dataset_heads_no_delete; DELETE FROM ip_dataset_heads WHERE kind='geoip'",
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let before = db.total_changes();
    assert_eq!(check(&mut db), Err(SchemaError::Integrity));
    assert_eq!(initialize(&mut db), Err(SchemaError::Integrity));
    assert_eq!(db.total_changes(), before);
}

fn insert_user(connection: &Connection) {
    connection.execute(
        "INSERT INTO users VALUES (x'00000000000000000000000000000001', 'user@example.test', '$argon2id$v=19$test', 1, 100, NULL)",
        [],
    ).unwrap();
}

#[test]
fn current_schema_checks_are_read_only_and_initialization_is_not_repeated() {
    let mut connection = database();
    insert_user(&connection);
    let changes = connection.total_changes();
    connection.pragma_update(None, "query_only", true).unwrap();
    check(&mut connection).unwrap();
    initialize(&mut connection).unwrap();
    assert_eq!(connection.total_changes(), changes);
    assert_eq!(identity(&connection).unwrap(), (APPLICATION_ID, VERSION));
}

#[test]
fn unknown_or_incomplete_schema_is_rejected_without_repair() {
    for mutation in [
        "PRAGMA application_id = 17",
        "PRAGMA user_version = 2",
        "DROP TRIGGER diagnostic_user_revoked; ALTER TABLE users DROP COLUMN disabled_at",
        "DROP INDEX user_sessions_user_id",
        "CREATE TRIGGER unexpected AFTER UPDATE ON users BEGIN SELECT 1; END",
        "ALTER TABLE users ADD COLUMN owner TEXT",
    ] {
        let mut connection = database();
        insert_user(&connection);
        connection.execute_batch(mutation).unwrap();
        let before = objects(&connection).unwrap();
        let identity_before = identity(&connection).unwrap();
        assert!(initialize(&mut connection).is_err());
        assert!(objects(&connection).unwrap() == before);
        assert_eq!(identity(&connection).unwrap(), identity_before);
        assert_eq!(
            connection
                .query_row("SELECT email FROM users", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "user@example.test"
        );
    }
    for sql in [
        "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('preserved')",
        "CREATE TABLE removed (value TEXT); DROP TABLE removed",
    ] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(sql).unwrap();
        assert_eq!(initialize(&mut connection), Err(SchemaError::Unsupported));
        assert_eq!(identity(&connection).unwrap(), (0, 0));
    }
}

#[test]
fn matching_version_and_columns_do_not_hide_missing_constraints() {
    for modified in [
        SQL.replace(" CHECK (length(id) = 16 AND id != zeroblob(16))", ""),
        SQL.replace(" AND id != zeroblob(16)", ""),
        SQL.replace(" REFERENCES users(id) ON DELETE RESTRICT", ""),
        SQL.replace("TEXT NOT NULL UNIQUE", "TEXT NOT NULL"),
        SQL.replace(") STRICT", ")"),
    ] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(&modified).unwrap();
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", VERSION)
            .unwrap();
        assert!(matches!(
            initialize(&mut connection),
            Err(SchemaError::Structure(_))
        ));
    }
}

#[test]
fn account_rows_enforce_identity_ranges_uniqueness_and_session_lifecycle() {
    let mut connection = database();
    insert_user(&connection);
    let session = "INSERT INTO user_sessions VALUES (x'00000000000000000000000000000002', zeroblob(32), x'00000000000000000000000000000001', 1, 100, NULL, NULL, 200, NULL, NULL)";
    connection.execute(session, []).unwrap();
    for sql in [
        "UPDATE users SET id = zeroblob(15)",
        "UPDATE users SET email = 'User@example.test'",
        "UPDATE users SET email = ' user@example.test'",
        "UPDATE users SET password_hash = 'plaintext'",
        "UPDATE users SET credential_version = 0",
        "UPDATE users SET credential_version = 9007199254740992",
        "UPDATE users SET credential_version = 'not-a-number'",
        "UPDATE users SET created_at = -1",
        "UPDATE users SET disabled_at = 99",
        "INSERT INTO users SELECT randomblob(16), email, password_hash, 1, 100, NULL FROM users",
        "UPDATE user_sessions SET token_hash = zeroblob(31)",
        "UPDATE user_sessions SET user_id = randomblob(16)",
        "UPDATE user_sessions SET credential_version = 9007199254740992",
        "UPDATE user_sessions SET expires_at = created_at",
        "UPDATE user_sessions SET last_used_at = 99",
        "UPDATE user_sessions SET revoked_at = 99",
        "INSERT INTO user_sessions SELECT randomblob(16), token_hash, user_id, 1, 100, NULL, NULL, 200, NULL FROM user_sessions",
        "DELETE FROM users",
    ] {
        assert!(
            connection.execute(sql, []).is_err(),
            "constraint accepted: {sql}"
        );
    }
    connection
        .execute(
            "UPDATE users SET credential_version = 9007199254740991, disabled_at = 101",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE user_sessions SET last_used_at = 101, revoked_at = 102",
            [],
        )
        .unwrap();
    check(&mut connection).unwrap();
}

#[test]
fn nil_resource_ids_and_references_are_rejected_and_detected_in_existing_rows() {
    for mutation in [
        "UPDATE users SET id = zeroblob(16)",
        "UPDATE user_sessions SET id = zeroblob(16)",
        "UPDATE user_sessions SET user_id = zeroblob(16)",
    ] {
        let mut connection = database();
        insert_user(&connection);
        connection.execute("INSERT INTO user_sessions VALUES (x'00000000000000000000000000000002', zeroblob(32), x'00000000000000000000000000000001', 1, 100, NULL, NULL, 200, NULL, NULL)", []).unwrap();
        // Test the ID CHECK independently of FK enforcement, including the
        // reference column. FK constraints must not mask a missing nil check.
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        assert!(matches!(
            connection.execute(mutation, []),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_CHECK
        ));
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection.execute(mutation, []).unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
        connection.pragma_update(None, "query_only", true).unwrap();
        assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
    }
}

#[test]
fn full_checks_find_existing_constraint_and_foreign_key_corruption() {
    let mut connection = database();
    insert_user(&connection);
    connection.execute_batch("PRAGMA ignore_check_constraints = ON; UPDATE users SET credential_version = 0; PRAGMA ignore_check_constraints = OFF").unwrap();
    assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
    connection
        .execute_batch("UPDATE users SET credential_version = 1; PRAGMA foreign_keys = OFF")
        .unwrap();
    connection.execute("INSERT INTO user_sessions VALUES (x'00000000000000000000000000000002', zeroblob(32), x'00000000000000000000000000000003', 1, 100, NULL, NULL, 200, NULL, NULL)", []).unwrap();
    assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
}

#[test]
fn activities_enforce_account_scope_target_ownership_and_bounded_sequence() {
    let mut connection = database();
    insert_user(&connection);
    connection.execute("INSERT INTO users VALUES (x'00000000000000000000000000000002', 'other@example.test', '$argon2id$v=19$test', 1, 100, NULL)", []).unwrap();
    connection.execute("INSERT INTO user_sessions VALUES (x'00000000000000000000000000000003', zeroblob(32), x'00000000000000000000000000000002', 1, 100, NULL, NULL, 200, NULL, NULL)", []).unwrap();
    connection.execute("INSERT INTO activity_events (id, scope, account_user_id, at, actor_user_id, action, target_user_id, request_id) VALUES (x'00000000000000000000000000000004', 'account', x'00000000000000000000000000000001', 100, x'00000000000000000000000000000001', 'account.register', x'00000000000000000000000000000001', x'00000000000000000000000000000005')", []).unwrap();
    for mutation in [
        "UPDATE activity_events SET scope = 'network'",
        "UPDATE activity_events SET sequence = 0",
        "UPDATE activity_events SET sequence = 9007199254740992",
        "UPDATE activity_events SET id = zeroblob(16)",
        "UPDATE activity_events SET request_id = zeroblob(16)",
        "UPDATE activity_events SET at = -1",
        "UPDATE activity_events SET actor_user_id = x'00000000000000000000000000000002'",
        "UPDATE activity_events SET target_user_id = x'00000000000000000000000000000002'",
        "UPDATE activity_events SET target_user_id = NULL",
        "UPDATE activity_events SET action = 'unknown'",
        "UPDATE activity_events SET action = 'session.login'",
        // A real session of another user cannot appear in this account's scope.
        "UPDATE activity_events SET action = 'session.login', target_user_id = NULL, target_session_id = x'00000000000000000000000000000003'",
    ] {
        assert!(
            connection.execute(mutation, []).is_err(),
            "accepted activity mutation: {mutation}"
        );
    }
    check(&mut connection).unwrap();
}
