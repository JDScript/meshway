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
async fn transactional_lifecycle_races_activities_and_cancelled_delivery() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executor = Executor::open(
        temp.path().canonicalize().unwrap().join("db"),
        Options::default(),
        (),
        schema::initialize,
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
            Err(AccountError::InvalidSession)
        ));
        assert!(matches!(
            accounts.logout(token.expose(), request()).await,
            Err(AccountError::InvalidSession)
        ));
        handle.execute(|c| c.execute_batch("UPDATE users SET disabled_at = NULL, credential_version = 1; UPDATE user_sessions SET created_at = 1, expires_at = 4102444800, revoked_at = NULL").unwrap()).await.unwrap();
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
            assert!(matches!(login(c, candidate, Duration::from_secs(86400), request()), Err(AccountError::InvalidCredentials)));
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
        assert!(matches!(accounts.login("user@example.test".into(), Secret::new("valid password".into()), request()).await, Err(AccountError::InvalidCredentials)));
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
        Err(AccountError::Storage)
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
        Err(AccountError::Storage)
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
        login(c, candidate, Duration::from_secs(86400), request())
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
        AccountError::Storage.to_string(),
        "account storage operation failed"
    );
    schema_check(&handle).await;
    executor.shutdown().await.unwrap();
}

async fn schema_check(handle: &crate::store::Handle) {
    handle.execute(schema::check).await.unwrap().unwrap();
}
