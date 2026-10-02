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
                "invalid current controller schema; refusing repair"
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

#[tokio::test]
async fn startup_rejects_same_name_membership_ignore_trigger_before_registration() {
    // Preserve the real register path, but reject this unknown owned schema
    // before a serving router can commit account/invitation without membership.
    for trigger_first in [true, false] {
        let f = Fixture::new();
        let base = base(&f).await;
        let owner = f.original_user().await;
        let email = "collision-member@fixture.example";
        let invitation = invite(&f, &base, &owner, email, "member").await;
        let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let index:String=db.query_row("SELECT sql FROM sqlite_schema WHERE type='index' AND name='account_active_membership'",[],|r|r.get(0)).unwrap();
        let Fixture {
            directory,
            app,
            admin: _,
        } = f;
        drop(app);
        if trigger_first {
            db.execute_batch("DROP INDEX account_active_membership")
                .unwrap();
        }
        db.execute_batch("CREATE TRIGGER account_active_membership BEFORE INSERT ON account_memberships BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        if trigger_first {
            db.execute_batch(&index).unwrap();
        }
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='account_active_membership'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        let definitions: Vec<(String, String, Option<String>)> = db
            .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        match AppState::open(directory.path()) {
            Err(meshway_controller::Error::Configuration(
                "invalid current controller schema; refusing repair",
            )) => {}
            Ok(state) => {
                let app = router(state);
                let (status,_)=request(&app,"POST","/api/v1/auth/invitations/register",None,Some(json!({"email":email,"password":"member password secret","invitation_token":invitation["invitation_token"]}))).await;
                let members:i64=db.query_row("SELECT COUNT(*) FROM account_memberships m JOIN account_users u ON u.user_id=m.user_id WHERE u.email=?1",[email],|r|r.get(0)).unwrap();
                panic!(
                    "schema gate accepted collision: register status={status}, actual memberships={members}"
                );
            }
            Err(_) => panic!("unexpected startup failure category"),
        }
        let after: Vec<(String, String, Option<String>)> = db
            .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(definitions, after);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM account_users WHERE email=?1",
                [email],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM account_invitations WHERE consumed_at IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM account_memberships", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
