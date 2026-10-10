use super::*;

#[test]
fn live_spec_closes_real_event_roots_and_resolves_every_dependency() {
    let spec = serde_json::to_value(http::openapi()).unwrap();
    fn references(value: &serde_json::Value, spec: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str) {
                    assert!(
                        spec.pointer(reference.strip_prefix('#').unwrap()).is_some(),
                        "unresolved {reference}"
                    );
                }
                for value in object.values() {
                    references(value, spec);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    references(value, spec);
                }
            }
            _ => {}
        }
    }
    references(&spec, &spec);
    // Nodes have no event stream (they long-poll sync); users keep theirs.
    assert!(spec["paths"].get("/agent-api/v1/events").is_none());
    for (path, event, scheme) in [(
        "/api/v1/networks/{networkId}/events",
        "UserEvent",
        "userSession",
    )] {
        let operation = &spec["paths"][path]["get"];
        assert_eq!(
            operation["responses"]["200"]["content"]["text/event-stream"]["schema"]["type"],
            "string"
        );
        assert_eq!(
            operation["x-meshway-sse"]["eventSchema"]["$ref"],
            format!("#/components/schemas/{event}")
        );
        assert!(operation["security"][0][scheme].is_array());
        for branch in spec["components"]["schemas"][event]["oneOf"]
            .as_array()
            .unwrap()
        {
            assert_eq!(branch["additionalProperties"], false);
        }
    }
    assert!(
        spec["components"]["securitySchemes"]
            .get("nodeCredential")
            .is_none()
    );
    for invalid in [
        r#"[]"#,
        r#"{"type":"sync.ready","batch":"b","cursor":"c","heartbeatIntervalMs":100,"disconnectAfterMs":200,"extra":1}"#,
        r#"{"type":"config.snapshot","batch":"b","configuration":{"type":"member","networkId":"00000000-0000-0000-0000-000000000001","revision":1,"nodes":[]},"progress":null}"#,
    ] {
        assert!(serde_json::from_str::<meshway_protocol::UserEvent>(invalid).is_err());
    }
    assert!(serde_json::from_str::<SystemState>(r#"["host","linux","x86","1"]"#).is_err());
}

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

#[tokio::test]
async fn production_reports_snapshots_delta_replay_permissions_and_shutdown() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("live"), true).await;
    let c = client();
    let owner = register(&c, &s, "live-owner@example.test", "valid password").await;
    let member = register(&c, &s, "live-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "live")
        .await
        .network
        .id;
    join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let (node, node_token, lease) = enroll(&c, &s, network, member.token.expose(), 5).await;
    let owner_node = enroll(&c, &s, network, owner.token.expose(), 6).await.0;
    let before = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&before.configuration);
    input
        .groups
        .insert("group:private".into(), vec![owner.user.email.clone()]);
    input.nodes.get_mut(&owner_node.node_id).unwrap().listeners = serde_json::from_str(r#"{"private":{"type":"vless","listenPort":24443,"tls":{"serverName":"private.example.test"},"stream":"websocket","websocket":{"path":"/private","responseHeaders":{"x-private":"PRIVATE-HEADER-VALUE"}}}}"#).unwrap();
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: before.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    let mut events =
        Events::open(&c, net(&s, network, "events"), member.token.expose(), None).await;
    let (_, first, raw) = events.user().await;
    let StreamEvent::ConfigSnapshot {
        configuration: UserConfigurationView::Member { nodes, .. },
        progress,
        ..
    } = first.0
    else {
        panic!("member projection")
    };
    assert_eq!(nodes.len(), 1);
    assert!(nodes.contains_key(&node.node_id));
    assert_eq!(progress.unwrap().scope, ProgressScope::Partial);
    for forbidden in [
        "group:private",
        "PRIVATE-HEADER-VALUE",
        "privateKey",
        &owner.user.email,
    ] {
        assert!(!raw.contains(forbidden), "{forbidden}");
    }
    let (_, entities, _) = events.user().await;
    let StreamEvent::EntitiesSnapshot { entities, .. } = entities.0 else {
        panic!("entities")
    };
    assert_eq!(entities.members.len(), 1);
    assert!(!entities.nodes[&node.node_id].can_delete);
    let inventory: NodeInventoryResponse = json(
        c.get(net(&s, network, "nodes"))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(entities.nodes[&node.node_id], inventory.items[0]);
    events.ready().await;
    let accepted: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&node_token)
            .json(&report(&lease, 1)),
    )
    .await;
    assert!(!accepted.duplicate);
    let (cursor, delta, _) = events.user().await;
    let StreamEvent::StateDelta {
        updated,
        as_of,
        sequence,
        configuration_revision,
        ..
    } = delta.0
    else {
        panic!("report delta")
    };
    assert_eq!(updated[&node.node_id].report_sequence.unwrap().get(), 1);
    let cursor = cursor.unwrap();
    let view: StateView = json(
        c.get(net(&s, network, &format!("nodes/{}/state", node.node_id)))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(view.nodes[&node.node_id].report_sequence.unwrap().get(), 1);
    assert_eq!(sequence, Some(view.sequence));
    assert_eq!(configuration_revision, Some(view.configuration_revision));
    assert!(as_of.unwrap() <= view.as_of);
    assert_eq!(
        updated[&node.node_id].observed_at,
        view.nodes[&node.node_id].observed_at
    );
    let peers: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&node_token)
            .json(&StateQuery { nodes: None }),
    )
    .await;
    assert!(peers.nodes.contains_key(&node.node_id));
    let _: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&node_token)
            .json(&report(&lease, 2)),
    )
    .await;
    let (second, _, _) = events.user().await;
    drop(events);
    let mut events = Events::open(
        &c,
        net(&s, network, "events"),
        member.token.expose(),
        Some(&cursor),
    )
    .await;
    let (replayed, event, _) = events.user().await;
    assert_eq!(replayed, second);
    assert!(matches!(event.0, StreamEvent::StateDelta { .. }));
    let mut unknown = Events::open(
        &c,
        net(&s, network, "events"),
        member.token.expose(),
        Some("unavailable-cursor"),
    )
    .await;
    assert!(matches!(
        unknown.user().await.1.0,
        StreamEvent::Reset { .. }
    ));
    unknown.ready().await;
    let synced: meshway_protocol::SyncResponse = json(
        c.post(format!("{}/agent-api/v1/sync", s.url))
            .bearer_auth(&node_token)
            .json(&serde_json::json!({})),
    )
    .await;
    let meshway_protocol::SyncNetmap::Full { netmap } = &synced.netmap else {
        panic!("full netmap")
    };
    assert_eq!(netmap.netmap.node_id, node.node_id);
    let now = chrono::Utc::now();
    let history = meshway_protocol::observations::HistoryReport {
        instance_id: lease.instance_id,
        events: vec![meshway_model::observations::ObservationEvent {
            event_id: EventId::new(*request().as_uuid()).unwrap(),
            netmap: netmap.reference(),
            occurred_at: now,
            payload: meshway_model::observations::ObservationPayload::CollectionGap {
                dropped: Sequence::new(2).unwrap(),
                from: now - chrono::Duration::seconds(2),
                to: now - chrono::Duration::seconds(1),
            },
        }],
    };
    let history_accepted: meshway_protocol::observations::HistoryAccepted = json(
        c.post(format!("{}/agent-api/v1/history", s.url))
            .bearer_auth(&node_token)
            .json(&history),
    )
    .await;
    assert_eq!(history_accepted.accepted, 1);
    let history_duplicate: meshway_protocol::observations::HistoryAccepted = json(
        c.post(format!("{}/agent-api/v1/history", s.url))
            .bearer_auth(&node_token)
            .json(&history),
    )
    .await;
    assert_eq!(history_duplicate.duplicates, 1);
    let page: meshway_protocol::observations::HistoryPage = json(
        c.post(net(&s, network, "history/query"))
            .bearer_auth(member.token.expose())
            .json(&meshway_protocol::observations::HistoryQuery {
                from: now - chrono::Duration::minutes(1),
                to: now + chrono::Duration::seconds(1),
                source_node_id: Some(node.node_id),
                peer_node_id: None,
                mode: meshway_protocol::observations::HistoryMode::Raw,
                bucket_seconds: None,
                cursor: None,
                limit: 20,
                kinds: None,
                link_id: None,
            }),
    )
    .await;
    let meshway_protocol::observations::HistoryItems::Raw { items } = page.result else {
        panic!("raw history")
    };
    assert_eq!(items.len(), 1);
    {
        let token = member.token.expose();
        assert_eq!(
            c.get(net(&s, network, "events"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    c.post(s.auth("logout"))
        .bearer_auth(member.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    for stream in [&mut events, &mut unknown] {
        assert!(matches!(
            stream.user().await.1.0,
            StreamEvent::Closed {
                reason: meshway_protocol::events::ClosedReason::SessionExpired
            }
        ));
    }
    assert!(
        events
            .frame::<meshway_protocol::UserEvent>()
            .await
            .is_none()
    );
    assert!(
        unknown
            .frame::<meshway_protocol::UserEvent>()
            .await
            .is_none()
    );
    tokio::time::timeout(Duration::from_secs(5), s.close())
        .await
        .unwrap();
}
