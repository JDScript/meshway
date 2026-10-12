use super::*;

#[tokio::test]
async fn production_config_changes_and_scope_changes_do_not_become_identity_loss() {
    use meshway_server::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("changes"), true).await;
    let c = client();
    let owner = register(&c, &s, "changes-owner@example.test", "valid password").await;
    let member = register(&c, &s, "changes-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "changes")
        .await
        .network
        .id;
    let member_id = join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let service = s.app.user_state_service().unwrap();
    let identity = |token: &str| ReadIdentity::user_session(token, network).unwrap();
    let member_stream = service
        .open_stream(identity(member.token.expose()), None, request())
        .await
        .unwrap();
    // Do not poll this initial batch: role loss must discard even its first
    // queued privileged configuration, not merely terminate after draining it.
    let response = c
        .post(net(&s, network, &format!("members/{member_id}/role")))
        .bearer_auth(owner.token.expose())
        .json(&meshway_protocol::ChangeMemberRoleRequest {
            expected_version: Version::MIN,
            role: MembershipRole::Member,
        })
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    assert!(matches!(
        service.poll(&member_stream).await.unwrap(),
        StreamPoll::Closed
    ));
    let owner_stream = service
        .open_stream(identity(owner.token.expose()), None, request())
        .await
        .unwrap();
    let member_stream = service
        .open_stream(identity(member.token.expose()), None, request())
        .await
        .unwrap();
    for stream in [&owner_stream, &member_stream] {
        for _ in 0..4 {
            assert!(matches!(
                service.poll(stream).await.unwrap(),
                StreamPoll::Frame(_)
            ));
        }
    }
    let mut http_events =
        Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
    http_events.ready().await;
    for pure_source in [false, true] {
        let current = source(&c, &s, network, owner.token.expose()).await;
        let mut input = current.configuration.clone();
        if !pure_source {
            input
                .groups
                .insert("group:hidden".into(), vec![owner.user.email.clone()]);
        }
        let submitted = request();
        let applied: ConfigurationAppliedResponse = json(
            c.post(net(&s, network, "configuration"))
                .bearer_auth(owner.token.expose())
                .json(&ConfigurationRequest::Document {
                    expected_revision: current.revision,
                    request_id: submitted,
                    source_jsonc: WireSecret::new(format!(
                        "// source-only={pure_source}\n{}",
                        serde_json::to_string(&input).unwrap()
                    )),
                }),
        )
        .await;
        loop {
            let (_, event, _) = http_events.user().await;
            match event.0 {
                StreamEvent::ConfigurationChanged {
                    revision, actor, ..
                } => {
                    assert_eq!(revision, applied.revision);
                    assert!(
                        matches!(actor, meshway_protocol::events::EventActor::User { user_id } if user_id == owner.user.id)
                    );
                    break;
                }
                StreamEvent::Reset { .. } => {
                    panic!("ordinary manager configuration changes retain HTTP stream continuity")
                }
                _ => {}
            }
        }
        let StreamPoll::Frame(frame) = service.poll(&owner_stream).await.unwrap() else {
            panic!("ordinary edit must not close stream")
        };
        let event: meshway_protocol::UserEvent = serde_json::from_str(&frame.data).unwrap();
        let StreamEvent::ConfigurationChanged {
            revision,
            actor,
            changed_pointers,
            ..
        } = event.0
        else {
            panic!("manager gets committed notice: {}", frame.data)
        };
        assert_eq!(revision, applied.revision);
        assert!(matches!(
            actor,
            meshway_protocol::events::EventActor::User { .. }
        ));
        assert_eq!(changed_pointers.is_empty(), pure_source);
        let StreamPoll::Frame(frame) = service.poll(&member_stream).await.unwrap() else {
            panic!("member notice")
        };
        let event: meshway_protocol::UserEvent = serde_json::from_str(&frame.data).unwrap();
        let StreamEvent::ConfigurationChanged {
            actor,
            changed_pointers,
            changes_complete,
            request_id,
            ..
        } = event.0
        else {
            panic!("member notice")
        };
        assert!(matches!(
            actor,
            meshway_protocol::events::EventActor::Redacted
        ));
        assert!(changed_pointers.is_empty());
        assert!(!changes_complete);
        assert!(request_id.is_none());
        for stream in [&owner_stream, &member_stream] {
            loop {
                match service.poll(stream).await.unwrap() {
                    StreamPoll::Frame(_) => {}
                    StreamPoll::Pending => break,
                    StreamPoll::Closed => panic!("configuration is not credential loss"),
                }
            }
        }
    }
    let queued = service
        .open_stream(identity(owner.token.expose()), None, request())
        .await
        .unwrap();
    for _ in 0..4 {
        assert!(matches!(
            service.poll(&queued).await.unwrap(),
            StreamPoll::Frame(_)
        ));
    }
    let current = source(&c, &s, network, owner.token.expose()).await;
    let first: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// first queued edit\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let StreamPoll::Frame(frame) = service.poll(&queued).await.unwrap() else {
        panic!("first notice")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::ConfigurationChanged { .. }
    ));
    let second: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: first.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// second queued edit\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    loop {
        let StreamPoll::Frame(frame) = service.poll(&queued).await.unwrap() else {
            panic!("drain still-authorized manager queue")
        };
        match serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0
        {
            StreamEvent::ConfigurationChanged { revision, .. } => {
                assert_eq!(revision, second.revision);
                break;
            }
            StreamEvent::Reset { .. } => {
                panic!("a complete manager view retains ordered ordinary pending frames")
            }
            _ => {}
        }
    }
    // A write during an unfinished full sync discards that batch and sends a
    // new authorized snapshot. The old cursor never becomes ready.
    let initial = service
        .open_stream(identity(owner.token.expose()), None, request())
        .await
        .unwrap();
    assert!(matches!(
        service.poll(&initial).await.unwrap(),
        StreamPoll::Frame(_)
    ));
    let current = source(&c, &s, network, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// pending change\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let StreamPoll::Frame(frame) = service.poll(&initial).await.unwrap() else {
        panic!("resynchronize initial batch")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::Reset { .. }
    ));
    // Logout also invalidates a fully pending replacement batch immediately.
    c.post(s.auth("logout"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let StreamPoll::Frame(frame) = service.poll(&initial).await.unwrap() else {
        panic!("closed reason")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::UserEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::Closed {
            reason: meshway_protocol::events::ClosedReason::SessionExpired
        }
    ));
    assert!(matches!(
        service.poll(&initial).await.unwrap(),
        StreamPoll::Closed
    ));
    s.close().await;
}
