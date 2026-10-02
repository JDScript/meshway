use super::*;

#[test]
fn startup_rejects_partial_account_install_without_repairing_or_enrolling() {
    // No login/KDF beyond normal startup is needed to recognize an incomplete
    // schema. Each fresh private fixture preserves the original data/version.
    for damage in [
        "DROP TRIGGER ownership_new_node; DROP TRIGGER device_account_binding_immutable; DROP TRIGGER device_account_binding_no_delete;",
        "DROP INDEX account_active_membership;",
        "DROP TABLE account_password_resets;",
        "DROP TRIGGER ownership_new_node; CREATE TRIGGER ownership_new_node AFTER INSERT ON nodes BEGIN SELECT 1; END;",
    ] {
        let f = Fixture::new();
        let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
        let before: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        db.execute_batch(damage).unwrap();
        let definitions: Vec<(String, Option<String>)> = db
            .prepare("SELECT name,sql FROM sqlite_schema ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let Fixture {
            directory,
            app,
            admin: _,
        } = f;
        drop(app);
        assert!(matches!(
            AppState::open(directory.path()),
            Err(meshway_controller::Error::Configuration(
                "incomplete or incompatible account integration schema"
            ))
        ));
        let after: Vec<(String, Option<String>)> = db
            .prepare("SELECT name,sql FROM sqlite_schema ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(definitions, after);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
