use super::*;

#[tokio::test]
async fn reacquired_instance_preserves_exact_target_but_requires_its_own_saved_and_applied() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("reacquire"), true).await;
    let c = client();
    let owner = register(&c, &s, "reacquire@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "reacquire")
        .await
        .network
        .id;
    let (node, token, first) = enroll(&c, &s, network, owner.token.expose(), 41).await;
    let current = source(&c, &s, network, owner.token.expose()).await;
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: request(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::SetNodeSettings {
                        node_id: node.node_id,
                        fields: meshway_configuration::NodeSettingsPatch {
                            remark: meshway_configuration::Input::Value("fixed target".into()),
                            ..Default::default()
                        },
                    },
                ],
            }),
    )
    .await;
    let url = net(
        &s,
        network,
        &format!("configuration/revisions/{}/progress", applied.revision),
    );
    let initial: ConfigurationApplyProgress =
        json(c.get(&url).bearer_auth(owner.token.expose())).await;
    let target = initial.nodes[&node.node_id].target.clone();
    let _: InstanceLease = json(
        c.post(format!("{}/agent-api/v1/instances/renew", s.url))
            .bearer_auth(&token)
            .json(&InstanceRequest {
                instance_id: first.instance_id,
                generation: first.generation,
            }),
    )
    .await;
    let receipt = |lease: &InstanceLease, phase, sequence| ReceiptRequest {
        instance: InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        target: target.clone(),
        phase,
        attempt: Version::MIN,
        sequence: Sequence::new(sequence).unwrap(),
        reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        failure: None,
    };
    for (phase, sequence) in [(ReceiptPhase::Saved, 0), (ReceiptPhase::Applied, 1)] {
        c.post(format!("{}/agent-api/v1/configuration/receipts", s.url))
            .bearer_auth(&token)
            .json(&receipt(&first, phase, sequence))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let mut conflicting = receipt(&first, ReceiptPhase::Saved, 0);
    conflicting.reported_at_ms += 2000;
    let conflict = c
        .post(format!("{}/agent-api/v1/configuration/receipts", s.url))
        .bearer_auth(&token)
        .json(&conflicting)
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert!(matches!(
        conflict
            .json::<meshway_protocol::ErrorResponse>()
            .await
            .unwrap()
            .problem,
        meshway_protocol::Problem::ReceiptContentConflict { .. }
    ));
    let replacement: InstanceLease = json(
        c.post(format!("{}/agent-api/v1/instances", s.url))
            .bearer_auth(&token)
            .json(&AcquireInstanceRequest {
                instance_id: InstanceId::new(*request().as_uuid()).unwrap(),
                capabilities: BTreeMap::from([(
                    "transport:native_udp".into(),
                    Capability::Transport {
                        roles: vec![TransportRole::Dial, TransportRole::Listen],
                        streams: vec![StreamKind::Udp],
                    },
                )]),
            }),
    )
    .await;
    assert_eq!(replacement.target.as_ref(), Some(&target));
    let waiting: ConfigurationApplyProgress =
        json(c.get(&url).bearer_auth(owner.token.expose())).await;
    assert_eq!(
        waiting.nodes[&node.node_id].phase,
        ApplicationPhase::Applied
    );
    assert_eq!(
        waiting.nodes[&node.node_id].instance_id,
        Some(first.instance_id)
    );
    assert_eq!(
        waiting.nodes[&node.node_id].freshness,
        ReceiptFreshness::AwaitingInstance
    );
    let endpoint = format!("{}/agent-api/v1/configuration/receipts", s.url);
    for (request, code) in [
        (
            receipt(&replacement, ReceiptPhase::Applied, 1),
            "receipt_precondition",
        ),
        (
            receipt(&first, ReceiptPhase::Applied, 1),
            "receipt_instance_mismatch",
        ),
        (
            {
                let mut r = receipt(&replacement, ReceiptPhase::Saved, 0);
                r.target.digest = meshway_model::Digest::from_bytes([0; 32]);
                r
            },
            "receipt_target_mismatch",
        ),
        (
            {
                let mut r = receipt(&replacement, ReceiptPhase::Saved, 0);
                r.reported_at_ms = 0;
                r
            },
            "receipt_precondition",
        ),
    ] {
        let response = c
            .post(&endpoint)
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: meshway_protocol::ErrorResponse = response.json().await.unwrap();
        assert_eq!(serde_json::to_value(body.problem).unwrap()["code"], code);
    }
    for (phase, sequence, expected) in [
        (ReceiptPhase::Saved, 0, ApplicationPhase::Saved),
        (ReceiptPhase::Applied, 1, ApplicationPhase::Applied),
    ] {
        c.post(&endpoint)
            .bearer_auth(&token)
            .json(&receipt(&replacement, phase, sequence))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let current: ConfigurationApplyProgress =
            json(c.get(&url).bearer_auth(owner.token.expose())).await;
        assert_eq!(current.nodes[&node.node_id].target, target);
        assert_eq!(
            current.nodes[&node.node_id].instance_id,
            Some(replacement.instance_id)
        );
        assert_eq!(current.nodes[&node.node_id].phase, expected);
        assert_eq!(
            current.nodes[&node.node_id].freshness,
            ReceiptFreshness::Current
        );
    }
    s.close().await;
}

#[tokio::test]
async fn production_demand_receipts_peer_expansion_and_node_termination() {
    use meshway_configuration::execution::{BusinessProtocol, BusinessTarget};
    use meshway_server::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(
        &dir.path().canonicalize().unwrap().join("execution-events"),
        true,
    )
    .await;
    let c = client();
    let owner = register(&c, &s, "execution-owner@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "execution-events")
        .await
        .network
        .id;
    let (a, at, ai) = enroll(&c, &s, network, owner.token.expose(), 11).await;
    let (b, bt, bi) = enroll(&c, &s, network, owner.token.expose(), 12).await;
    let current = source(&c, &s, network, owner.token.expose()).await;
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&current.configuration);
    for node in [a.node_id, b.node_id] {
        input.nodes.get_mut(&node).unwrap().listeners =
            serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#).unwrap();
    }
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
    let progress_url = net(
        &s,
        network,
        &format!("configuration/revisions/{}/progress", applied.revision),
    );
    let fixed: ConfigurationApplyProgress =
        json(c.get(&progress_url).bearer_auth(owner.token.expose())).await;
    assert_eq!(fixed.summary.target_count, 2);
    let mut progress_events =
        Events::open(&c, net(&s, network, "events"), owner.token.expose(), None).await;
    progress_events.ready().await;
    let original = fixed.nodes[&a.node_id].target.clone();
    let instance = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    for (phase, sequence) in [(ReceiptPhase::Saved, 0), (ReceiptPhase::Applied, 1)] {
        empty(
            c.post(format!("{}/agent-api/v1/configuration/receipts", s.url))
                .bearer_auth(&at)
                .json(&ReceiptRequest {
                    instance: instance.clone(),
                    target: original.clone(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None,
                }),
        )
        .await;
        if phase == ReceiptPhase::Saved {
            let _: InstanceLease = json(
                c.post(format!("{}/agent-api/v1/instances/renew", s.url))
                    .bearer_auth(&at)
                    .json(&instance),
            )
            .await;
        }
    }
    loop {
        let (_, event, _) = progress_events.user().await;
        match event.0 {
            StreamEvent::ConfigurationProgress { progress } if progress.summary.applied == 1 => {
                let queried: ConfigurationApplyProgress =
                    json(c.get(&progress_url).bearer_auth(owner.token.expose())).await;
                assert!(progress == queried);
                assert_eq!(progress.nodes[&a.node_id].target, original);
                assert_eq!(
                    progress.nodes[&a.node_id].freshness,
                    ReceiptFreshness::Current
                );
                break;
            }
            StreamEvent::Reset { .. } => panic!("receipt is data, not manager authorization loss"),
            _ => {}
        }
    }
    let mut stream = Events::open(&c, format!("{}/agent-api/v1/events", s.url), &at, None).await;
    for _ in 0..4 {
        stream.frame::<meshway_protocol::NodeEvent>().await.unwrap();
    }
    let demand: DemandResponse = json(
        c.post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: instance.clone(),
                request_id: request(),
                target: BusinessTarget::Mesh { node: b.node_id },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            }),
    )
    .await;
    let mut target = None;
    loop {
        let (_, event, _) = stream
            .frame::<meshway_protocol::NodeEvent>()
            .await
            .expect("demand is not credential loss");
        match event.0 {
            StreamEvent::ConfigSnapshot { configuration, .. } => target = configuration.target,
            StreamEvent::Ready { .. } => break,
            _ => {}
        }
    }
    let target = target.unwrap();
    assert_eq!(target.execution_publication, demand.publication);
    let inventory: NodeInventoryView = json(
        c.get(net(&s, network, &format!("nodes/{}", a.node_id)))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(inventory.desired_execution.as_ref(), Some(&target));
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&at)
            .json(&StateQuery {
                nodes: Some(vec![b.node_id]),
            }),
    )
    .await;
    assert!(peer.nodes.contains_key(&b.node_id));
    for (phase, sequence) in [(ReceiptPhase::Saved, 0), (ReceiptPhase::Applied, 1)] {
        empty(
            c.post(format!("{}/agent-api/v1/configuration/receipts", s.url))
                .bearer_auth(&at)
                .json(&ReceiptRequest {
                    instance: instance.clone(),
                    target: target.clone(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None,
                }),
        )
        .await;
    }
    let mut source_report = report(&ai, 1);
    source_report.state.configuration.saved = Some(target.clone());
    source_report.state.configuration.applied = Some(target.clone());
    let observed_at = source_report.observed_at;
    source_report.state.sockets.insert(
        "udp-socket".into(),
        SocketState {
            local_endpoints: None,
            resource: ResourceRef::Listener { key: "udp".into() },
            configuration: Some(target.clone()),
            generation: Sequence::new(1).unwrap(),
            network: SocketNetwork::Udp,
            bound_address: "0.0.0.0:23456".parse().unwrap(),
            endpoints: vec![
                EndpointCandidate {
                    address: "192.0.2.10:23456".into(),
                    origin: EndpointSource::Interface {
                        interface: "private-interface-fixture".into(),
                    },
                    observed_at,
                    expires_at: observed_at + chrono::Duration::seconds(30),
                },
                EndpointCandidate {
                    address: "203.0.113.20:40000".into(),
                    origin: EndpointSource::Stun {
                        observer: "192.0.2.1:3478".into(),
                    },
                    observed_at,
                    expires_at: observed_at + chrono::Duration::seconds(30),
                },
            ],
        },
    );
    source_report.state.listeners.insert(
        "udp".into(),
        ListenerState {
            phase: ListenerPhase::Listening,
            socket_ids: vec!["udp-socket".into()],
            loaded_certificate_sha256: None,
            problem: None,
            configuration: Some(target.clone()),
        },
    );
    let _: StateAccepted = json(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&at)
            .json(&source_report),
    )
    .await;
    let (_, event, _) = stream.frame::<meshway_protocol::NodeEvent>().await.unwrap();
    assert!(matches!(event.0, StreamEvent::StateDelta { .. }));
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&bt)
            .json(&StateQuery {
                nodes: Some(vec![a.node_id]),
            }),
    )
    .await;
    let VisibleReport::Peer {
        report: peer_report,
    } = peer.nodes[&a.node_id].report.as_ref().unwrap()
    else {
        panic!("peer projection")
    };
    // Visibility alone cannot disclose probe targets while the querying Node
    // still holds its pre-demand execution. Obtain the current grant first.
    assert!(peer_report.endpoints.is_empty());
    let _: meshway_protocol::NodeConfigurationResponse = json(
        c.post(format!("{}/agent-api/v1/configuration/refresh", s.url))
            .bearer_auth(&bt)
            .json(&meshway_protocol::NodeConfigurationRequest {
                instance: InstanceRequest {
                    instance_id: bi.instance_id,
                    generation: bi.generation,
                },
                base: None,
            }),
    )
    .await;
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&bt)
            .json(&StateQuery {
                nodes: Some(vec![a.node_id]),
            }),
    )
    .await;
    let Some(VisibleReport::Peer {
        report: peer_report,
    }) = &peer.nodes[&a.node_id].report
    else {
        panic!("current peer projection")
    };
    assert_eq!(peer_report.endpoints.len(), 2);
    let peer_json = serde_json::to_string(&peer).unwrap();
    for private in [
        "private-interface-fixture",
        "0.0.0.0:23456",
        "192.0.2.1:3478",
    ] {
        assert!(!peer_json.contains(private));
    }
    // A self-report cannot turn an interface observation into a configured
    // endpoint absent from the Controller's exact published listener values.
    let mut invalid = source_report.clone();
    invalid.sequence = Sequence::new(2).unwrap();
    invalid
        .state
        .sockets
        .get_mut("udp-socket")
        .unwrap()
        .endpoints[0]
        .origin = EndpointSource::Configured {
        config_path: format!("/nodes/{}/listeners/udp/endpoints/0", a.node_id),
    };
    assert_eq!(
        c.post(format!("{}/agent-api/v1/state", s.url))
            .bearer_auth(&at)
            .json(&invalid)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    // A source-only revision does not invalidate the still-eligible earlier
    // target's peer dependencies (the target revision is deliberately older).
    let current = source(&c, &s, network, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = json(
        c.post(net(&s, network, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(format!(
                    "// comment\n{}",
                    current.source_jsonc.expose()
                )),
            }),
    )
    .await;
    let peer: StateView = json(
        c.post(format!("{}/agent-api/v1/state/query", s.url))
            .bearer_auth(&at)
            .json(&StateQuery {
                nodes: Some(vec![b.node_id]),
            }),
    )
    .await;
    assert!(peer.nodes.contains_key(&b.node_id));
    // Use the production poll boundary to deterministically leave an initial
    // batch queued while the real HTTP key-revocation transaction commits.
    let node_service = s.app.node_state_service().unwrap();
    let queued = node_service
        .open_stream(
            ReadIdentity::Node {
                authorization: bt.clone(),
            },
            None,
            request(),
        )
        .await
        .unwrap();
    empty(
        c.post(net(
            &s,
            network,
            &format!("nodes/{}/keys/revoke", b.node_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&RevokeNodeKeyRequest {
            key_id: b.keys.control.id,
            expected_generation: b.keys.control.generation,
        }),
    )
    .await;
    let StreamPoll::Frame(frame) = node_service.poll(&queued).await.unwrap() else {
        panic!("node credential closed")
    };
    assert!(matches!(
        serde_json::from_str::<meshway_protocol::NodeEvent>(&frame.data)
            .unwrap()
            .0,
        StreamEvent::Closed { .. }
    ));
    assert!(matches!(
        node_service.poll(&queued).await.unwrap(),
        StreamPoll::Closed
    ));
    empty(
        c.post(format!("{}/agent-api/v1/instances/release", s.url))
            .bearer_auth(&at)
            .json(&instance),
    )
    .await;
    // Drain only frames already emitted before the release. The transport must
    // terminate, independently of its keepalive connection or a new request.
    while stream
        .frame::<meshway_protocol::NodeEvent>()
        .await
        .is_some()
    {}
    s.close().await;
}
