use super::*;
use meshway_configuration::netmap::apply_netmap_delta;

fn short_hold(app: &mut App, hold: Duration) {
    app.netmap_timing.hold = hold;
}

#[tokio::test]
async fn sync_answers_full_delta_or_unchanged_and_renews_only_the_lease() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, mut app, network, a, b, ak, _, signer_a, _) = fixture().await;
    short_hold(&mut app, Duration::from_millis(200));
    let owner = owner(&app).await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let service = app.netmaps();

    let first = service
        .sync(token.token.expose(), holding(None))
        .await
        .unwrap();
    let SyncNetmap::Full { netmap: base } = first.netmap else {
        panic!("expected full");
    };
    assert_eq!(base.netmap.node_id, a);
    assert_eq!(first.lease.netmap, base.reference());
    assert_eq!(
        first.lease.expires_at_ms - first.lease.issued_at_ms,
        24 * 60 * 60 * 1000
    );
    let cursor = first
        .peer_facts
        .as_ref()
        .expect("initial fact replacement")
        .cursor;

    // Holding the current netmap: the hold times out unchanged, with a later
    // lease and no stored write.
    let stored = rows(&app, network).await;
    let started = Instant::now();
    let renewed = service
        .sync(
            token.token.expose(),
            SyncRequest {
                netmap: Some(base.reference()),
                state: Some(cursor),
            },
        )
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert_eq!(renewed.netmap, SyncNetmap::Unchanged);
    assert_eq!(renewed.lease.netmap, base.reference());
    assert!(renewed.lease.issued_at_ms >= first.lease.issued_at_ms);
    assert_eq!(rows(&app, network).await, stored);

    // The held base is the delta base of the next serial.
    edit(&app, &owner, network, vec![rename(b, "renamed")])
        .await
        .unwrap();
    let changed = service
        .sync(token.token.expose(), holding(Some(base.reference())))
        .await
        .unwrap();
    let SyncNetmap::Delta { delta } = changed.netmap else {
        panic!("expected delta");
    };
    let next = apply_netmap_delta(&base, *delta).unwrap();
    assert_eq!(next.reference(), changed.lease.netmap);
    assert_eq!(next, load(&app, network, a).await);

    // Any base other than the current one or its predecessor gets full content.
    edit(&app, &owner, network, vec![rename(b, "again")])
        .await
        .unwrap();
    let stale = service
        .sync(token.token.expose(), holding(Some(base.reference())))
        .await
        .unwrap();
    let SyncNetmap::Full { netmap } = stale.netmap else {
        panic!("expected full for a stale base");
    };
    assert_eq!(netmap.netmap.serial.get(), base.netmap.serial.get() + 2);
    // Another Node's netmap reference is never a base: the caller gets its own content.
    let foreign = current(&app, network, b).await.unwrap().current;
    let SyncNetmap::Full { netmap } = service
        .sync(token.token.expose(), holding(Some(foreign)))
        .await
        .unwrap()
        .netmap
    else {
        panic!("expected full for a foreign base");
    };
    assert_eq!(netmap.netmap.node_id, a);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_held_sync_wakes_on_its_change_and_not_on_unrelated_commits() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, mut app, network, a, _, ak, _, signer_a, _) = fixture().await;
    short_hold(&mut app, Duration::from_millis(1_500));
    let owner = owner(&app).await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let base = current(&app, network, a).await.unwrap().current;

    let pending = spawn_sync(&app, &token, holding(Some(base)));
    held(&app, 1).await;
    comment_only_edit(&app, &owner, network).await;
    let (response, elapsed) = pending.await.unwrap();
    assert_eq!(response.unwrap().netmap, SyncNetmap::Unchanged);
    assert!(elapsed >= Duration::from_millis(1_300), "{elapsed:?}");

    app.netmap_timing.hold = Duration::from_secs(50);
    let pending = spawn_sync(&app, &token, holding(Some(base)));
    held(&app, 1).await;
    let committed = Instant::now();
    edit(
        &app,
        &owner,
        network,
        vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
    )
    .await
    .unwrap();
    let (response, _) = pending.await.unwrap();
    assert!(committed.elapsed() < Duration::from_secs(1));
    let response = response.unwrap();
    assert!(matches!(response.netmap, SyncNetmap::Delta { .. }));
    assert_eq!(
        response.lease.netmap,
        current(&app, network, a).await.unwrap().current
    );
    assert_eq!(app.effects.netmaps.pending(), 0);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_sync_of_the_same_node_replaces_the_first() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let execution = app.execution();
    let at = session(&execution, network, a, &ak, &signer_a).await;
    let bt = session(&execution, network, b, &bk, &signer_b).await;
    let a_base = current(&app, network, a).await.unwrap().current;
    let b_base = current(&app, network, b).await.unwrap().current;

    let first = spawn_sync(&app, &at, holding(Some(a_base)));
    let other = spawn_sync(&app, &bt, holding(Some(b_base)));
    held(&app, 2).await;
    let second = spawn_sync(&app, &at, holding(Some(a_base)));
    let (response, elapsed) = first.await.unwrap();
    assert_eq!(response.unwrap().netmap, SyncNetmap::Unchanged);
    assert!(elapsed < Duration::from_secs(5));
    held(&app, 2).await;
    assert!(!second.is_finished() && !other.is_finished());
    // Shutdown answers every held request.
    (app.stream_shutdown())();
    for joined in [second, other] {
        assert!(matches!(
            joined.await.unwrap().0,
            Ok(_)
                | Err(crate::error::Error::Unavailable(
                    crate::error::Unavailability::Closed
                ))
        ));
    }
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn http_sync_requires_a_current_node_session() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, mut app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    short_hold(&mut app, Duration::from_millis(100));
    let owner = owner(&app).await;
    let token = session(&app.execution(), network, a, &ak, &signer_a).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let sync = |bearer: Option<String>, body: serde_json::Value| {
        let mut request = client.post(format!("{base}/agent-api/v1/sync")).json(&body);
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        request.send()
    };

    let response = sync(Some(token.token.expose().to_owned()), serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: SyncResponse = response.json().await.unwrap();
    let SyncNetmap::Full { netmap } = &body.netmap else {
        panic!("expected full");
    };
    assert_eq!(
        (netmap.netmap.network_id, netmap.netmap.node_id),
        (network, a)
    );
    assert!(body.check(&holding(None)).is_ok());
    let unchanged = sync(
        Some(token.token.expose().to_owned()),
        serde_json::json!({ "netmap": netmap.reference() }),
    )
    .await
    .unwrap();
    assert_eq!(unchanged.status(), 200);
    assert_eq!(
        unchanged.json::<SyncResponse>().await.unwrap().netmap,
        SyncNetmap::Unchanged
    );

    // Unknown fields, missing, user and forged bearers are rejected.
    for (bearer, body, status) in [
        (
            Some(token.token.expose().to_owned()),
            serde_json::json!({ "node": b }),
            400,
        ),
        (None, serde_json::json!({}), 401),
        (Some(owner.clone()), serde_json::json!({}), 401),
        (Some("mwn_forged".to_owned()), serde_json::json!({}), 401),
    ] {
        assert_eq!(
            sync(bearer, body).await.unwrap().status(),
            status,
            "{status}"
        );
    }

    // A revoked Node: its session ends with its credentials.
    app.execution()
        .revoke_key(
            &owner,
            network,
            a,
            meshway_protocol::node::RevokeNodeKeyRequest {
                key_id: ak.control.id,
                expected_generation: ak.control.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        sync(Some(token.token.expose().to_owned()), serde_json::json!({}))
            .await
            .unwrap()
            .status(),
        401
    );
    // Revoking either credential retires its HTTP sessions and netmap.
    let bt = session(&app.execution(), network, b, &bk, &signer_b).await;
    app.execution()
        .revoke_key(
            &owner,
            network,
            b,
            meshway_protocol::node::RevokeNodeKeyRequest {
                key_id: bk.dataplane.id,
                expected_generation: bk.dataplane.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        sync(Some(bt.token.expose().to_owned()), serde_json::json!({}))
            .await
            .unwrap()
            .status(),
        401
    );
    server.abort();
    app.shutdown().await.unwrap();
}
