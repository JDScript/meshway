use super::*;

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    initialize(&mut connection).unwrap();
    connection
}

fn insert_user(connection: &Connection) {
    connection.execute(
        "INSERT INTO users VALUES (zeroblob(16), 'user@example.test', '$argon2id$v=19$test', 1, 100, NULL)",
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
        "ALTER TABLE users DROP COLUMN disabled_at",
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
        SQL.replace(" CHECK (length(id) = 16)", ""),
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
        assert_eq!(initialize(&mut connection), Err(SchemaError::Structure));
    }
}

#[test]
fn account_rows_enforce_identity_ranges_uniqueness_and_session_lifecycle() {
    let mut connection = database();
    insert_user(&connection);
    let session = "INSERT INTO user_sessions VALUES (zeroblob(16), zeroblob(32), zeroblob(16), 1, 100, NULL, NULL, 200, NULL)";
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
fn full_checks_find_existing_constraint_and_foreign_key_corruption() {
    let mut connection = database();
    insert_user(&connection);
    connection.execute_batch("PRAGMA ignore_check_constraints = ON; UPDATE users SET credential_version = 0; PRAGMA ignore_check_constraints = OFF").unwrap();
    assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
    connection
        .execute_batch("UPDATE users SET credential_version = 1; PRAGMA foreign_keys = OFF")
        .unwrap();
    connection.execute("INSERT INTO user_sessions VALUES (zeroblob(16), zeroblob(32), randomblob(16), 1, 100, NULL, NULL, 200, NULL)", []).unwrap();
    assert_eq!(check(&mut connection), Err(SchemaError::Integrity));
}
