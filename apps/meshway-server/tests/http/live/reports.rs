use super::*;

#[tokio::test]
async fn disconnected_streams_release_active_quota_and_keep_bounded_replay() {
    use meshway_server::error::Error;
    use meshway_server::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(
        &dir.path().canonicalize().unwrap().join("stream-quota"),
        true,
    )
    .await;
    let c = client();
    let owner = register(&c, &s, "stream-quota@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "streams")
        .await
        .network
        .id;
    let service = s.app.user_state_service().unwrap();
    let identity = ReadIdentity::user_session(owner.token.expose(), network).unwrap();
    let mut last_cursor = None;
    for _ in 0..12 {
        let id = service
            .open_stream(identity.clone(), None, request())
            .await
            .unwrap();
        for _ in 0..4 {
            let StreamPoll::Frame(frame) = service.poll(&id).await.unwrap() else {
                panic!("initial batch")
            };
            if frame.id.is_some() {
                last_cursor = frame.id;
            }
        }
        service.cancel(&id);
    }
    let mut active = vec![];
    for _ in 0..4 {
        active.push(
            service
                .open_stream(identity.clone(), None, request())
                .await
                .unwrap(),
        );
    }
    assert!(matches!(
        service.open_stream(identity.clone(), None, request()).await,
        Err(Error::StateLimit)
    ));
    assert!(matches!(
        service
            .open_stream(identity.clone(), last_cursor.clone(), request())
            .await,
        Err(Error::StateLimit)
    ));
    service.cancel(&active.pop().unwrap());
    let resumed = service
        .open_stream(identity, last_cursor, request())
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&resumed).await.unwrap(),
        StreamPoll::Pending
    ));
    service.cancel(&resumed);
    for id in active {
        service.cancel(&id);
    }
    // The real response body's Drop must reach cancel, too; refreshes should
    // not require waiting for the one-minute replay retention deadline.
    for _ in 0..10 {
        let mut events =
            Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
        events.ready().await;
        drop(events);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    s.close().await;
}
