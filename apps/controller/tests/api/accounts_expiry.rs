use super::*;
use std::time::Duration;

// Preserve Astra's real-lock expiry boundary: +2 seconds, 300ms confirmed
// pending, release at expiry+1, below the unchanged five-second busy timeout.
async fn expires_during_writer_wait(kind: &str) {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.original_user().await;
    let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
    let expires = chrono::Utc::now().timestamp() + 2;
    db.execute("UPDATE account_sessions SET expires_at=?1", [expires])
        .unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (path, body) = match kind {
        "account" => (
            format!("{base}/invitations"),
            json!({"email":"pending@review.test","role":"owner","expires_in_seconds":3600}),
        ),
        "device" => (
            format!("{base}/device-authorizations"),
            json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_challenge":s256(&"v".repeat(43)),"name":"review-device","agent_version":"review"}),
        ),
        "network" => (
            format!("{base}/enrollment-tokens"),
            json!({"expires_in_seconds":3600}),
        ),
        _ => unreachable!(),
    };
    let app = f.app.clone();
    let token = owner.clone();
    let pending =
        tokio::spawn(async move { request(&app, "POST", &path, Some(&token), Some(body)).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!pending.is_finished());
    while chrono::Utc::now().timestamp() < expires + 1 {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    db.execute_batch("COMMIT").unwrap();
    assert_eq!(pending.await.unwrap().0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        request(&f.app, "GET", "/api/v1/auth/session", Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for table in [
        "account_invitations",
        "device_account_bindings",
        "enrollment_tokens",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_mutation_authenticates_after_writer_wait() {
    expires_during_writer_wait("account").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_approval_authenticates_after_writer_wait() {
    expires_during_writer_wait("device").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_mutation_keeps_post_begin_authentication() {
    expires_during_writer_wait("network").await;
}
