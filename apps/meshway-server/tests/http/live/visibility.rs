use super::*;

#[tokio::test]
async fn member_inventory_excludes_execution_peers_and_settings_refresh_in_stream() {
    use meshway_configuration::execution::{BusinessProtocol, BusinessTarget};
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("inventory"), true).await;
    let c = client();
    let owner = register(&c, &s, "inventory-owner@example.test", "valid password").await;
    let member = register(&c, &s, "inventory-member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "inventory")
        .await
        .network
        .id;
    let membership = join(
        &c,
        &s,
        network,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let (a, at, ai) = enroll(&c, &s, network, member.token.expose(), 31).await;
    let (b, _, _) = enroll(&c, &s, network, owner.token.expose(), 32).await;
    let current = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
    input
        .tags_owners
        .insert("tag:peer".into(), vec![owner.user.email.clone()]);
    input.nodes.get_mut(&b.node_id).unwrap().identity =
        serde_json::from_str(r#"{"type":"tag","tags":["tag:peer"]}"#).unwrap();
    for node in input.nodes.values_mut() {
        node.listeners =
            serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#).unwrap();
    }
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    let _: DemandResponse = json(
        c.post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: InstanceRequest {
                    instance_id: ai.instance_id,
                    generation: ai.generation,
                },
                request_id: request(),
                target: BusinessTarget::Mesh { node: b.node_id },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            }),
    )
    .await;
    let mut events =
        Events::open(&c, net(&s, network, "events"), member.token.expose(), None).await;
    let stream_request_id = events.response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let StreamEvent::ConfigSnapshot { request_id, .. } = events.user().await.1.0 else {
        panic!("configuration")
    };
    assert_eq!(request_id.to_string(), stream_request_id);
    let StreamEvent::EntitiesSnapshot { entities, .. } = events.user().await.1.0 else {
        panic!("inventory")
    };
    let inventory: NodeInventoryResponse = json(
        c.get(net(&s, network, "nodes"))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(
        entities.nodes.values().cloned().collect::<Vec<_>>(),
        inventory.items
    );
    assert_eq!(entities.nodes.len(), 1);
    assert!(entities.nodes.contains_key(&a.node_id));
    let StreamEvent::StateSnapshot { state, .. } = events.user().await.1.0 else {
        panic!("state")
    };
    assert!(
        state.nodes.contains_key(&b.node_id),
        "authorized narrow peer State remains available"
    );
    // The peer is named by its configured hostname, nothing more from its
    // system report.
    assert_eq!(state.nodes[&b.node_id].hostname, "node-32");
    assert_eq!(state.nodes[&a.node_id].hostname, "node-31");
    assert!(matches!(
        state.nodes[&b.node_id].report,
        None | Some(VisibleReport::Peer { .. })
    ));
    events.ready().await;
    let mut node_events =
        Events::open(&c, format!("{}/agent-api/v1/events", s.url), &at, None).await;
    node_events
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .unwrap();
    let StreamEvent::EntitiesSnapshot { entities, .. } = node_events
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .unwrap()
        .1
        .0
    else {
        panic!("node inventory")
    };
    assert_eq!(entities.nodes[&b.node_id].keys.dataplane, b.keys.dataplane);
    let current = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
    input.nodes.get_mut(&a.node_id).unwrap().hostname = "updated-member-host".into();
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    let mut refreshed = false;
    loop {
        match events.user().await.1.0 {
            StreamEvent::ConfigSnapshot {
                configuration:
                    UserConfigurationView::Member {
                        nodes, revision, ..
                    },
                ..
            } => {
                assert_eq!(revision, applied.revision);
                assert_eq!(nodes[&a.node_id].hostname, "updated-member-host");
                assert_eq!(nodes.len(), 1);
                refreshed = true;
            }
            StreamEvent::Ready { .. } => {
                assert!(refreshed);
                break;
            }
            _ => {}
        }
    }
    let mut manager =
        Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
    manager.user().await;
    let StreamEvent::EntitiesSnapshot { entities, .. } = manager.user().await.1.0 else {
        panic!("manager inventory")
    };
    assert_eq!(entities.nodes.len(), 2);
    assert!(entities.nodes.values().all(|node| node.can_delete));
    drop(events);
    let mut reconnected =
        Events::open(&c, net(&s, network, "events"), member.token.expose(), None).await;
    let StreamEvent::ConfigSnapshot {
        configuration: UserConfigurationView::Member { nodes, .. },
        ..
    } = reconnected.user().await.1.0
    else {
        panic!("reconnected settings")
    };
    assert_eq!(nodes[&a.node_id].hostname, "updated-member-host");
    reconnected.ready().await;
    c.post(format!("{}/agent-api/v1/instances/release", s.url))
        .bearer_auth(&at)
        .json(&InstanceRequest {
            instance_id: ai.instance_id,
            generation: ai.generation,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    reconnected.ready().await;
    // An offline node has no changing live execution reference. Its settings
    // must refresh even when the restricted disclosure digest stays identical.
    let current = source(&c, &s, network, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: request(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::SetNodeSettings {
                        node_id: a.node_id,
                        fields: meshway_configuration::NodeSettingsPatch {
                            remark: meshway_configuration::Input::Value(
                                "offline settings changed".into(),
                            ),
                            ..Default::default()
                        },
                    },
                ],
            }),
    )
    .await;
    let mut refreshed = false;
    loop {
        match reconnected.user().await.1.0 {
            StreamEvent::ConfigSnapshot {
                configuration: UserConfigurationView::Member { nodes, .. },
                ..
            } => {
                assert_eq!(nodes[&a.node_id].remark, "offline settings changed");
                refreshed = true;
            }
            StreamEvent::Ready { .. } => {
                assert!(refreshed);
                break;
            }
            _ => {}
        }
    }
    let current = source(&c, &s, network, owner.token.expose()).await;
    c.post(net(&s, network, &format!("members/{membership}/remove")))
        .bearer_auth(owner.token.expose())
        .json(&meshway_protocol::RemoveMemberRequest {
            expected_version: Version::MIN,
            expected_revision: current.revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    loop {
        match reconnected.user().await.1.0 {
            StreamEvent::Closed {
                reason: meshway_protocol::events::ClosedReason::MembershipRevoked,
            } => break,
            StreamEvent::EntitiesDelta { .. }
            | StreamEvent::StateDelta { .. }
            | StreamEvent::ConfigurationProgress { .. } => {}
            _ => panic!("membership loss must discard queued private settings"),
        }
    }
    assert!(
        reconnected
            .frame::<meshway_protocol::UserEvent>()
            .await
            .is_none()
    );
    assert_eq!(
        c.get(net(&s, network, "events"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    s.close().await;
}

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
        let mut input =
            meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
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
