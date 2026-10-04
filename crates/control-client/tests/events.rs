mod support;

use meshway_control_client::{
    ApplyConfigurationPath, ClientError, EventDelivery, EventStream, GetConfigurationSourcePath,
    GetConfigurationSourceQuery, StreamOptions, WatchNetworkPath,
};
use meshway_protocol::{
    ConfigurationRequest, CreateNetworkRequest, ErrorResponse, RegisterRequest, UserEvent,
    WireSecret, events::StreamEvent, live::UserConfigurationView,
};
use std::time::Duration;

async fn next(stream: &mut EventStream<UserEvent, ErrorResponse>) -> EventDelivery<UserEvent> {
    tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("event deadline")
        .expect("live event stream")
        .expect("typed event")
}

#[tokio::test]
async fn generated_rust_stream_decodes_production_snapshots_and_replays_only_unapplied_changes() {
    let (app, client, server, _directory) = support::controller(true).await;
    let session = client
        .register(&RegisterRequest {
            email: "rust-events@example.test".into(),
            password: WireSecret::new("valid stream password".into()),
        })
        .await
        .unwrap();
    let token = session.token.expose();
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
                name: "Rust stream".into(),
            },
            token,
        )
        .await
        .unwrap()
        .network;
    let source = client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: network.id,
            },
            &GetConfigurationSourceQuery::default(),
            token,
        )
        .await
        .unwrap();
    let path = WatchNetworkPath {
        network_id: network.id,
    };
    let mut stream = client
        .watch_network(&path, token, &StreamOptions::default())
        .await
        .unwrap();
    let config = next(&mut stream).await;
    assert!(config.id.is_none());
    let StreamEvent::ConfigSnapshot {
        batch,
        configuration,
        progress,
    } = config.data.0
    else {
        panic!("expected configuration snapshot");
    };
    let UserConfigurationView::Manager {
        network_id,
        revision,
        ..
    } = configuration
    else {
        panic!("expected manager scope");
    };
    assert_eq!(network_id, network.id);
    assert_eq!(revision, source.revision);
    assert_eq!(progress.unwrap().summary.target_count, 0);
    let entities = next(&mut stream).await;
    assert!(entities.id.is_none());
    let StreamEvent::EntitiesSnapshot {
        batch: entity_batch,
        entities,
    } = entities.data.0
    else {
        panic!("expected entity snapshot");
    };
    assert_eq!(entity_batch, batch);
    assert_eq!(entities.members.len(), 1);
    assert!(entities.nodes.is_empty());
    let state = next(&mut stream).await;
    assert!(state.id.is_none());
    let StreamEvent::StateSnapshot {
        batch: state_batch,
        state,
    } = state.data.0
    else {
        panic!("expected state snapshot");
    };
    assert_eq!(state_batch, batch);
    assert_eq!(state.network_id, network.id);
    assert!(state.nodes.is_empty());
    let ready = next(&mut stream).await;
    let StreamEvent::Ready {
        batch: ready_batch,
        cursor,
        ..
    } = ready.data.0
    else {
        panic!("expected complete sync");
    };
    assert_eq!(ready_batch, batch);
    assert_eq!(ready.id.as_deref(), Some(cursor.as_str()));
    // Applying the complete initial batch is the only committed cursor. Read
    // the next notice but deliberately disconnect before applying it.
    let applied_cursor = cursor;
    let changed = client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: network.id,
            },
            &ConfigurationRequest::Document {
                request_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
                expected_revision: source.revision,
                source_jsonc: WireSecret::new(format!(
                    "{}\n// Rust stream change",
                    source.source_jsonc.expose()
                )),
            },
            token,
        )
        .await
        .unwrap();
    let mut notice_id = None;
    for _ in 0..12 {
        let event = next(&mut stream).await;
        if let StreamEvent::ConfigurationChanged {
            previous_revision,
            revision,
            source_changed,
            ..
        } = event.data.0
        {
            assert_eq!(previous_revision, source.revision);
            assert_eq!(revision, changed.revision);
            assert!(source_changed);
            notice_id = event.id;
            break;
        }
    }
    let notice_id = notice_id.expect("resumable configuration notice");
    drop(stream);
    let mut stream = client
        .watch_network(
            &path,
            token,
            &StreamOptions {
                applied_cursor: Some(&applied_cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut replayed = false;
    for _ in 0..12 {
        let event = next(&mut stream).await;
        assert!(!matches!(
            event.data.0,
            StreamEvent::ConfigSnapshot { .. } | StreamEvent::Reset { .. }
        ));
        if let StreamEvent::ConfigurationChanged { revision, .. } = event.data.0 {
            assert_eq!(revision, changed.revision);
            assert_eq!(event.id.as_deref(), Some(notice_id.as_str()));
            replayed = true;
            break;
        }
    }
    assert!(replayed);
    client.logout(token).await.unwrap();
    let mut closed = false;
    for _ in 0..12 {
        match tokio::time::timeout(Duration::from_secs(3), stream.next())
            .await
            .unwrap()
        {
            None
            | Some(Ok(EventDelivery {
                data: UserEvent(StreamEvent::Closed { .. }),
                ..
            })) => {
                closed = true;
                break;
            }
            Some(Ok(_)) => {}
            Some(Err(_)) => panic!("revocation must close a valid stream without invalid data"),
        }
    }
    assert!(closed);
    assert!(matches!(
        client
            .watch_network(&path, token, &StreamOptions::default())
            .await,
        Err(ClientError::Http {
            status: 401,
            problem: Some(_)
        })
    ));
    drop(stream);
    app.check_storage().await.unwrap();
    server.0.abort();
    app.shutdown().await.unwrap();
}
