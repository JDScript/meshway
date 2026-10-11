use super::*;

#[tokio::test]
async fn original_absolute_expiry_and_gc_never_issue_a_replacement_nonce() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b) = setup(true).await;
    let mut req = request(&app, &a, &b).await;
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 - 1;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 60_000;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 500;
    let first = offer(&app, &a, req.clone()).await;
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(attempts.as_slice().is_empty());
    assert_eq!(
        app.connectivity().prepare(&a.token, req).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_peer_session_deadline_recheck_is_terminal_even_if_storage_later_recovers() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b) = setup(true).await;
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    // The SQL owner shortens the real peer-session deadline after prepare.
    let node = b.id;
    let old = app
        .executor
        .handle()
        .execute(move |connection| {
            let old: i64 = connection
                .query_row(
                    "SELECT max(expires_at) FROM node_sessions WHERE node_id=?1",
                    [node.as_uuid().as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![
                        node.as_uuid().as_bytes().as_slice(),
                        chrono::Utc::now().timestamp_millis() + 100
                    ],
                )
                .unwrap();
            old
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.executor
        .handle()
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![node.as_uuid().as_bytes().as_slice(), old],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_restart_changes_epoch_and_old_credentials_and_attempts_cannot_reappear() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b) = setup(true).await;
    let req = request(&app, &a, &b).await;
    let old = offer(&app, &a, req.clone()).await;
    let keys = meshway_protocol::node::NodeKeyMetadata {
        control: a.netmap.netmap.self_.self_keys.control_key.clone(),
        dataplane: a.netmap.netmap.self_.self_keys.dataplane_key.clone(),
    };
    app.shutdown().await.unwrap();
    let app = App::open(crate::Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert!(matches!(
        app.connectivity().query(&a.token, a.query()).await,
        Err(crate::error::Error::InvalidCredentials)
    ));
    let signer = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let token = session(&app.execution(), network, a.id, &keys, &signer).await;
    let lease = app
        .execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    let netmap = a.reference();
    let QueryUdpTraversalResponse::Current {
        state_epoch,
        attempts,
        ..
    } = app
        .connectivity()
        .query(
            token.token.expose(),
            QueryUdpTraversalRequest {
                instance: instance.clone(),
                netmap,
            },
        )
        .await
        .unwrap()
    else {
        panic!();
    };
    assert_ne!(state_epoch, old.state_epoch);
    assert!(attempts.as_slice().is_empty());
    let mut req = req;
    req.instance = instance.clone();
    req.netmap = netmap;
    assert_eq!(
        app.connectivity()
            .prepare(token.token.expose(), req)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    assert_eq!(
        app.connectivity()
            .accept(
                token.token.expose(),
                old.attempt_id,
                UdpTraversalAttemptRequest {
                    instance,
                    netmap,
                    state_epoch: old.state_epoch,
                    version: old.version,
                    digest: old.digest
                }
            )
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    app.shutdown().await.unwrap();
}
