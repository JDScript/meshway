use std::{
    future::Future,
    sync::mpsc,
    task::{Context, Poll, Waker},
};

use argon2::{Argon2, PasswordHash, PasswordVerifier};

use super::*;
use crate::{
    accounts::{Accounts, Options as AccountOptions, Secret, TEST_KDF_LOCK},
    store::{Executor, Options, schema},
};

fn request() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

fn verify_candidate(candidate: &LoginCandidate) {
    // The same stored production PHC is actually verified before racing the
    // issuing transaction; this is not a fabricated "password matched" flag.
    Argon2::default()
        .verify_password(
            b"valid password",
            &PasswordHash::new(&candidate.password_hash).unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn session_use_and_reset_failures_follow_real_account_lifecycles() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let (executor, ()) = Executor::open(
        temp.path().canonicalize().unwrap().join("db"),
        Options::default(),
        (),
        |connection, _| schema::tests::initialize_fixture(connection),
    )
    .await
    .unwrap();
    let handle = executor.handle();
    let accounts = Accounts::new(
        handle.clone(),
        AccountOptions::new(true, Duration::from_secs(86400)).unwrap(),
    );
    let email = "metadata-reset@example.test";
    let issued = accounts
        .register_from_client(
            email.into(),
            Secret::new("valid password".into()),
            request(),
            Some("Firefox · Linux".into()),
        )
        .await
        .unwrap();
    let digest = crate::accounts::token_digest(issued.token.expose()).unwrap();
    let created = issued.session.created_at.timestamp_millis();
    let expires = issued.session.expires_at.timestamp_millis();
    handle
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let (used, label): (Option<i64>, Option<String>) = tx
                .query_row(
                    "SELECT last_used_at,client_label FROM user_sessions WHERE token_hash=?1",
                    [digest.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(used, None, "issuing a token does not prove subsequent use");
            assert_eq!(label.as_deref(), Some("Firefox · Linux"));
            for (at, expected) in [
                (created + 1, created + 1),
                (created + 1000, created + 1),
                (created + 31_001, created + 31_001),
            ] {
                current_session(&tx, &digest, at).unwrap();
                let used: i64 = tx
                    .query_row(
                        "SELECT last_used_at FROM user_sessions WHERE token_hash=?1",
                        [digest.as_slice()],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(used, expected);
            }
            assert!(matches!(
                current_session(&tx, &digest, expires),
                Err(Error::InvalidSession)
            ));
            let after: i64 = tx
                .query_row(
                    "SELECT last_used_at FROM user_sessions WHERE token_hash=?1",
                    [digest.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(after, created + 31_001);
            tx.commit().unwrap();
        })
        .await
        .unwrap();

    let first = accounts
        .issue_password_reset(email.into(), 60, request())
        .await
        .unwrap();
    let second = accounts
        .issue_password_reset(email.into(), 60, request())
        .await
        .unwrap();
    assert!(matches!(
        accounts.inspect_password_reset(first.token.expose()).await,
        Err(Error::ResetReplaced)
    ));
    let second_hash = crate::accounts::one_time_digest("mwr_", second.token.expose()).unwrap();
    handle
        .execute(move |db| {
            db.execute(
                "UPDATE password_resets SET expires_at=1 WHERE token_hash=?1",
                [second_hash.as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        accounts.inspect_password_reset(second.token.expose()).await,
        Err(Error::ResetExpired)
    ));
    let (unknown, _) = crate::accounts::issue_one_time("mwr_").unwrap();
    assert!(matches!(
        accounts.inspect_password_reset(unknown.expose()).await,
        Err(Error::InvalidReset)
    ));
    let third = accounts
        .issue_password_reset(email.into(), 60, request())
        .await
        .unwrap();
    accounts
        .redeem_password_reset(
            third.token.expose(),
            Secret::new("reset replacement password".into()),
            request(),
        )
        .await
        .unwrap();
    assert!(matches!(
        accounts.inspect_password_reset(third.token.expose()).await,
        Err(Error::ResetUsed)
    ));
    assert!(matches!(
        accounts
            .redeem_password_reset(
                third.token.expose(),
                Secret::new("another replacement password".into()),
                request()
            )
            .await,
        Err(Error::ResetUsed)
    ));
    assert!(matches!(
        accounts.authenticate(issued.token.expose()).await,
        Err(Error::InvalidSession)
    ));

    let fourth = accounts
        .issue_password_reset(email.into(), 60, request())
        .await
        .unwrap();
    let session = accounts
        .login(
            email.into(),
            Secret::new("reset replacement password".into()),
            request(),
        )
        .await
        .unwrap();
    accounts
        .change_password(
            session.token.expose(),
            Secret::new("reset replacement password".into()),
            Secret::new("latest replacement password".into()),
            request(),
        )
        .await
        .unwrap();
    assert!(matches!(
        accounts.inspect_password_reset(fourth.token.expose()).await,
        Err(Error::ResetReplaced)
    ));
    executor.shutdown().await.unwrap();
}

#[tokio::test]
async fn transactional_lifecycle_races_activities_and_cancelled_delivery() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let (executor, ()) = Executor::open(
        temp.path().canonicalize().unwrap().join("db"),
        Options::default(),
        (),
        |connection, _| schema::tests::initialize_fixture(connection),
    )
    .await
    .unwrap();
    let handle = executor.handle();
    let accounts = Accounts::new(
        handle.clone(),
        AccountOptions::new(true, Duration::from_secs(86400)).unwrap(),
    );
    let issued = accounts
        .register(
            "user@example.test".into(),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap();
    let token = issued.token;
    for mutation in [
        "UPDATE users SET disabled_at = created_at",
        "UPDATE users SET credential_version = credential_version + 1",
        "UPDATE user_sessions SET created_at = 1, expires_at = 2",
        "UPDATE user_sessions SET revoked_at = created_at",
    ] {
        handle
            .execute(move |connection| {
                connection.execute_batch(mutation).unwrap();
            })
            .await
            .unwrap();
        assert!(matches!(
            accounts.authenticate(token.expose()).await,
            Err(Error::InvalidSession)
        ));
        assert!(matches!(
            accounts.logout(token.expose(), request()).await,
            Err(Error::InvalidSession)
        ));
        handle.execute(|c| c.execute_batch("UPDATE users SET disabled_at = NULL, credential_version = 1; UPDATE user_sessions SET created_at = 1, expires_at = 4102444800000, revoked_at = NULL").unwrap()).await.unwrap();
    }
    // The final writer checks the state after the slow verification, including
    // PHC change without a version bump. No extra session or activity is issued.
    for mutation in [
        "UPDATE users SET disabled_at = created_at",
        "UPDATE users SET credential_version = 2",
        "UPDATE users SET password_hash = '$argon2id$v=19$changed'",
    ] {
        let candidate = handle
            .execute(|c| login_candidate(c, Some("user@example.test")))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        verify_candidate(&candidate);
        let old_hash = candidate.password_hash.clone();
        handle.execute(move |c| {
            c.execute_batch(mutation).unwrap();
            let tx = c.transaction().unwrap();
            assert!(matches!(login(&tx, candidate, Duration::from_secs(86400), request(), None), Err(Error::InvalidCredentials)));
            drop(tx);
            assert_eq!(c.query_row("SELECT count(*) FROM user_sessions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            assert_eq!(c.query_row("SELECT count(*) FROM activity_events", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            c.execute("UPDATE users SET disabled_at = NULL, credential_version = 1, password_hash = ?1", [old_hash.as_str()]).unwrap();
        }).await.unwrap();
    }
    // Malformed/oversize PHCs take the dummy verification, produce the same
    // public error, and never cause their supplied resource costs to be used.
    let original = handle
        .execute(|c| login_candidate(c, Some("user@example.test")))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for hash in ["$argon2id$v=19$m=4294967295,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(), format!("$argon2id$v=19${}", "x".repeat(500))] {
        handle.execute(move |c| c.execute("UPDATE users SET password_hash = ?1", [hash]).unwrap()).await.unwrap();
        assert!(matches!(accounts.login("user@example.test".into(), Secret::new("valid password".into()), request()).await, Err(Error::InvalidCredentials)));
    }
    handle
        .execute(move |c| {
            c.execute(
                "UPDATE users SET password_hash = ?1",
                [original.password_hash.as_str()],
            )
            .unwrap()
        })
        .await
        .unwrap();

    // Activity failure rolls back the session and account, and rolls back a
    // revocation. No success can escape without its durable activity.
    handle.execute(|c| c.execute_batch("CREATE TRIGGER reject_activity BEFORE INSERT ON activity_events BEGIN SELECT RAISE(ABORT, 'private internal reason'); END").unwrap()).await.unwrap();
    assert!(matches!(
        accounts.logout(token.expose(), request()).await,
        Err(Error::Storage(_))
    ));
    accounts.authenticate(token.expose()).await.unwrap();
    assert!(matches!(
        accounts
            .register(
                "failed@example.test".into(),
                Secret::new("valid password".into()),
                request()
            )
            .await,
        Err(Error::Storage(_))
    ));
    handle
        .execute(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            c.execute_batch("DROP TRIGGER reject_activity").unwrap();
        })
        .await
        .unwrap();
    let logged_in = accounts
        .login(
            "user@example.test".into(),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap();
    accounts
        .logout(logged_in.token.expose(), request())
        .await
        .unwrap();
    handle.execute(|c| {
        let rows = c.prepare("SELECT sequence, action, scope, account_user_id = actor_user_id, length(request_id) FROM activity_events WHERE account_user_id = (SELECT id FROM users) AND sequence > 0 AND sequence <= 3 ORDER BY sequence LIMIT 10").unwrap().query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?, r.get::<_, i64>(4)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 3);
        for ((seq, action, scope, same_actor, request_len), expected) in rows.iter().zip(["account.register", "session.login", "session.logout"]) {
            assert!(*seq > 0);
            assert_eq!(action, expected);
            assert_eq!(scope, "account");
            assert!(*same_actor);
            assert_eq!(*request_len, 16);
        }
    }).await.unwrap();

    let candidate = handle
        .execute(|c| login_candidate(c, Some("user@example.test")))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    verify_candidate(&candidate);
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, wait) = mpsc::channel();
    let mut login_future = Box::pin(handle.execute(move |c| {
        started.send(()).unwrap();
        wait.recv().unwrap();
        let tx = c.transaction().unwrap();
        let issued = login(&tx, candidate, Duration::from_secs(86400), request(), None);
        tx.commit().unwrap();
        issued
    }));
    assert!(matches!(
        login_future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    running.await.unwrap();
    drop(login_future);
    release.send(()).unwrap();
    handle
        .execute(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM user_sessions", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                3
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM activity_events", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                4
            );
        })
        .await
        .unwrap();
    // Error renderings are fixed even when SQLite supplied private text.
    assert_eq!(
        Error::from(rusqlite::Error::InvalidQuery).to_string(),
        "storage failed: row conversion failed"
    );
    schema_check(&handle).await;
    executor.shutdown().await.unwrap();
}

async fn schema_check(handle: &crate::store::Handle) {
    handle.execute(schema::check).await.unwrap().unwrap();
}

#[tokio::test]
async fn persisted_account_session_and_activity_times_are_unix_milliseconds() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let (executor, ()) = Executor::open(
        temp.path().canonicalize().unwrap().join("db"),
        Options::default(),
        (),
        |connection, _| schema::tests::initialize_fixture(connection),
    )
    .await
    .unwrap();
    let handle = executor.handle();
    let accounts = Accounts::new(
        handle.clone(),
        AccountOptions::new(true, Duration::from_secs(90)).unwrap(),
    );
    let before = Utc::now().timestamp_millis();
    let registered = accounts
        .register(
            "time@example.test".into(),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap();
    let after = Utc::now().timestamp_millis();
    let session = registered.session.clone();
    handle.execute(move |connection| {
        let (user_created, session_created, expires, activity_at) = connection.query_row(
            "SELECT u.created_at, s.created_at, s.expires_at, a.at FROM users u JOIN user_sessions s ON s.user_id = u.id JOIN activity_events a ON a.account_user_id = u.id WHERE s.id = ?1 AND a.action = 'account.register'",
            [session.session_id.as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
        ).unwrap();
        assert!((before..=after).contains(&user_created));
        assert_eq!(user_created, session_created);
        assert_eq!(activity_at, session_created);
        assert_eq!(session_created, session.created_at.timestamp_millis());
        assert_eq!(expires, session.expires_at.timestamp_millis());
        assert_eq!(expires - session_created, 90_000);
    }).await.unwrap();
    assert_eq!(
        accounts
            .authenticate(registered.token.expose())
            .await
            .unwrap(),
        registered.session
    );

    let before = Utc::now().timestamp_millis();
    let logged_in = accounts
        .login(
            "time@example.test".into(),
            Secret::new("valid password".into()),
            request(),
        )
        .await
        .unwrap();
    let after = Utc::now().timestamp_millis();
    let session = logged_in.session.clone();
    handle.execute(move |connection| {
        let (created, expires, activity_at, digest) = connection.query_row(
            "SELECT s.created_at, s.expires_at, a.at, s.token_hash FROM user_sessions s JOIN activity_events a ON a.target_session_id = s.id WHERE s.id = ?1 AND a.action = 'session.login'",
            [session.session_id.as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?)),
        ).unwrap();
        assert!((before..=after).contains(&created));
        assert_eq!(created, session.created_at.timestamp_millis());
        assert_eq!(activity_at, created);
        assert_eq!(expires, session.expires_at.timestamp_millis());
        assert_eq!(expires - created, 90_000);
        let digest: [u8; 32] = digest.try_into().unwrap();
        let transaction = connection.transaction().unwrap();
        assert_eq!(current_session(&transaction, &digest, expires - 1).unwrap(), session);
        assert_eq!(current_session(&transaction, &digest, expires), Err(Error::InvalidSession));
    }).await.unwrap();

    let before = Utc::now().timestamp_millis();
    accounts
        .logout(logged_in.token.expose(), request())
        .await
        .unwrap();
    let after = Utc::now().timestamp_millis();
    let session_id = logged_in.session.session_id;
    handle.execute(move |connection| {
        let (revoked, activity_at) = connection.query_row(
            "SELECT s.revoked_at, a.at FROM user_sessions s JOIN activity_events a ON a.target_session_id = s.id WHERE s.id = ?1 AND a.action = 'session.logout'",
            [session_id.as_uuid().as_bytes().as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        ).unwrap();
        assert!((before..=after).contains(&revoked));
        assert_eq!(activity_at, revoked);
    }).await.unwrap();

    // Real authentication must compare its fresh clock in the same unit as a
    // persisted millisecond expiry, not accidentally treat it as far-future seconds.
    let session_id = registered.session.session_id;
    handle
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE user_sessions SET created_at = 1, expires_at = ?1 WHERE id = ?2",
                    params![
                        Utc::now().timestamp_millis() - 1,
                        session_id.as_uuid().as_bytes().as_slice()
                    ],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert_eq!(
        accounts.authenticate(registered.token.expose()).await,
        Err(Error::InvalidSession)
    );
    assert_eq!(
        accounts.logout(registered.token.expose(), request()).await,
        Err(Error::InvalidSession)
    );
    schema_check(&handle).await;
    executor.shutdown().await.unwrap();
}
