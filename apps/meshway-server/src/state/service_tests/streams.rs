use super::*;

#[test]
fn sse_export_declares_bytes_and_a_separate_json_event_schema() {
    use utoipa::OpenApi as _;
    let mut spec = http::state::StateApi::openapi();
    let path = "/api/v1/networks/{networkId}/events";
    http::events::declare_sse::<TestEvent>(&mut spec, path).unwrap();
    let json = serde_json::to_value(spec).unwrap();
    assert_eq!(json["openapi"], "3.1.0");
    assert_eq!(
        json["paths"][path]["get"]["responses"]["200"]["content"]["text/event-stream"]["schema"]["type"],
        "string"
    );
    assert_eq!(
        json["paths"][path]["get"]["x-meshway-sse"]["format"],
        "json-data"
    );
    let reference = json["paths"][path]["get"]["x-meshway-sse"]["eventSchema"]["$ref"]
        .as_str()
        .unwrap();
    assert!(json.pointer(reference.trim_start_matches('#')).is_some());
    let header = json["paths"][path]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == "Last-Event-ID")
        .unwrap();
    assert_eq!(header["required"], false);
    assert_eq!(header["schema"]["type"], "string");
    assert_eq!(
        json["components"]["schemas"]["GeoPosition"]["properties"]["latitude"]["minimum"],
        -90
    );
}

#[tokio::test]
async fn pending_snapshot_is_discarded_on_role_change_and_shutdown_closes_streams() {
    use super::super::stream::{StreamPoll, StreamSource};
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let request =
        || meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap();
    let issued = app
        .accounts()
        .register(
            "scope@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            request(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(issued.token.expose(), "scope".into(), request())
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(issued.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    let network_id = network.id;
    service
        .handle
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE memberships SET role='member',version=version+1 WHERE network_id=?1",
                    [network_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Closed
    ));
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    assert!(service.poll(&subscription).await.is_err());
    assert!(service.open_stream(identity, None, rid()).await.is_err());
}

#[tokio::test]
async fn streams_wait_for_commits_and_deadlines_instead_of_polling() {
    use super::super::stream::{StreamPoll, StreamSource};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let issued = app
        .accounts()
        .register(
            "wake@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(issued.token.expose(), "wake".into(), rid())
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(issued.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service
        .open_stream(identity.clone(), None, rid())
        .await
        .unwrap();
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    // Silence: nothing wakes the stream.
    assert!(
        tokio::time::timeout(StdDuration::from_millis(300), service.wait(&subscription))
            .await
            .is_err()
    );
    // The session now ends in two seconds; the next projection picks that up
    // as its deadline.
    let digest = crate::accounts::token_digest(issued.token.expose()).unwrap();
    let ends = (Utc::now() + Duration::milliseconds(2000)).timestamp_millis();
    service
        .handle
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE user_sessions SET expires_at=?1 WHERE token_hash=?2",
                    params![ends, digest.as_slice()],
                )
                .unwrap();
        })
        .await
        .unwrap();
    // A committed change for this network wakes it at once, with a frame to show.
    let started = std::time::Instant::now();
    let renamed = app
        .networks()
        .rename(
            issued.token.expose(),
            network.id,
            "woken".into(),
            network.name.clone(),
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "woken");
    tokio::time::timeout(StdDuration::from_secs(1), service.wait(&subscription))
        .await
        .expect("a commit wakes the subscription");
    assert!(started.elapsed() < StdDuration::from_millis(500));
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    // A commit for another network does not.
    let other = app
        .networks()
        .create(issued.token.expose(), "elsewhere".into(), rid())
        .await
        .unwrap();
    assert_ne!(other.id, network.id);
    assert!(
        tokio::time::timeout(StdDuration::from_millis(300), service.wait(&subscription))
            .await
            .is_err()
    );
    // The session's own expiry is a deadline the subscription computed itself.
    tokio::time::timeout(StdDuration::from_secs(3), service.wait(&subscription))
        .await
        .expect("the session deadline wakes the subscription");
    assert!(Utc::now().timestamp_millis() >= ends);
    assert!(matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_) | StreamPoll::Closed
    ));
    // Shutdown wakes every waiter.
    let fresh = app
        .accounts()
        .login(
            "wake@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            rid(),
        )
        .await
        .unwrap();
    let identity = ReadIdentity::UserSession {
        digest: crate::accounts::token_digest(fresh.token.expose()).unwrap(),
        network: network.id,
    };
    let subscription = service.open_stream(identity, None, rid()).await.unwrap();
    while matches!(
        service.poll(&subscription).await.unwrap(),
        StreamPoll::Frame(_)
    ) {}
    let waiting = tokio::spawn({
        let service = service.clone();
        let subscription = subscription.clone();
        async move { tokio::time::timeout(StdDuration::from_secs(1), service.wait(&subscription)).await }
    });
    tokio::time::sleep(StdDuration::from_millis(50)).await;
    app.shutdown().await.unwrap();
    waiting
        .await
        .unwrap()
        .expect("shutdown wakes the subscription");
}

#[tokio::test]
async fn http_snapshot_cursor_replay_commit_cancellation_and_logout() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        temp.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let accounts = app.accounts();
    let issued = accounts
        .register(
            "owner@example.com".into(),
            crate::accounts::Secret::new("correct horse battery".into()),
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let token = issued.token.expose().to_owned();
    let network = app
        .networks()
        .create(
            &token,
            "初始 network".into(),
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let service = app
        .state_service(SqlProvider {
            sealer: app.sealer(),
        })
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/api/v1/networks/{}/events",
        listener.local_addr().unwrap(),
        network.id
    );
    let router =
        http::state::user_router(service.clone()).merge(http::state::node_router(service.clone()));
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .no_proxy()
        .build()
        .unwrap();
    let mut stream = open(&client, &url, &token, None).await;
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::ConfigSnapshot { .. })
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::EntitiesSnapshot { .. })
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        (None, TestEvent::StateSnapshot { .. })
    ));
    let (ready, event) = stream.next().await.unwrap();
    assert!(matches!(event, TestEvent::Ready { .. }));
    let ready = ready.unwrap();
    let runtime = Arc::clone(&service.runtime);
    let handle = service.handle.clone();
    let network_id = network.id;
    let (committed, committed_rx) = oneshot::channel();
    let mutation = tokio::spawn(async move {
        handle
            .execute(move |connection| {
                let tx = connection.transaction().unwrap();
                tx.execute(
                    "UPDATE networks SET name='committed after client cancellation' WHERE id=?1",
                    [network_id.as_uuid().as_bytes().as_slice()],
                )
                .unwrap();
                tx.commit().unwrap();
                runtime.apply(Effect::Authorization { network_id });
                let _ = committed.send(());
            })
            .await
    });
    committed_rx.await.unwrap();
    mutation.abort();
    let (changed, event) = stream.next().await.unwrap();
    let changed = changed.unwrap();
    match event {
        TestEvent::EntitiesDelta {
            base_cursor,
            updated: entities,
            ..
        } => {
            assert_eq!(base_cursor, ready);
            assert_eq!(entities.name, "committed after client cancellation");
        }
        _ => panic!("expected entity replacement"),
    }
    drop(stream);
    let mut resumed = open(&client, &url, &token, Some(&ready)).await;
    let (replayed, event) = resumed.next().await.unwrap();
    assert_eq!(replayed, Some(changed));
    assert!(matches!(event, TestEvent::EntitiesDelta { .. }));
    accounts
        .logout(
            &token,
            meshway_model::RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    loop {
        let (_, event) = resumed.next().await.expect("explicit session closure");
        if matches!(
            event,
            TestEvent::Closed {
                reason: meshway_protocol::events::ClosedReason::SessionExpired
            }
        ) {
            break;
        }
        assert!(matches!(event, TestEvent::StateDelta { .. }));
    }
    assert!(resumed.next().await.is_none());
    drop(resumed);
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}
