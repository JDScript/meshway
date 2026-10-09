use super::*;

#[tokio::test]
async fn node_get_decodes_required_uri_scalars_without_relaxing_json_or_instance_authorization() {
    let server = Server::new().await;
    let user = server.user("node-uri-query@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
        })
        .await;
    let request = demand(&a, "b");
    let created: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &request),
        201,
    )
    .await;
    let path = selected(&request);
    let read: NodeObservationResponse =
        json(server.get(&path, a.token.expose()).query(&query(&a)), 200).await;
    assert_eq!(read.task.id, created.task.id);
    assert_eq!(
        read.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    let instance = a.instance.instance_id;
    let generation = a.instance.generation;
    for suffix in [
        String::new(),
        format!("?instanceId={instance}"),
        format!("?generation={generation}"),
        format!("?instanceId=&generation={generation}"),
        format!("?instanceId=not-a-uuid&generation={generation}"),
        format!("?instanceId=00000000-0000-0000-0000-000000000000&generation={generation}"),
        format!("?instanceId={instance}&generation={generation}&generation={generation}"),
        format!("?instanceId={instance}&instanceId={instance}&generation={generation}"),
        format!("?instanceId={instance}&generation={generation}&unknown=1"),
    ] {
        status(
            server.get(&format!("{path}{suffix}"), a.token.expose()),
            400,
        )
        .await;
    }
    for malformed in [
        "",
        "zero",
        "0",
        "-1",
        "%2B1",
        "%201",
        "1.0",
        "1e0",
        "9007199254740992",
        "18446744073709551616",
    ] {
        status(
            server.get(
                &format!("{path}?instanceId={instance}&generation={malformed}"),
                a.token.expose(),
            ),
            400,
        )
        .await;
    }
    // Valid URI scalars reach the existing instance authorization check.
    for foreign_generation in [generation.checked_next().unwrap(), Version::MAX] {
        status(
            server
                .get(&path, a.token.expose())
                .query(&NodeObservationQuery {
                    instance_id: instance,
                    generation: foreign_generation,
                }),
            401,
        )
        .await;
    }
    status(
        server
            .get(&path, a.token.expose())
            .query(&NodeObservationQuery {
                instance_id: b.instance.instance_id,
                generation,
            }),
        401,
    )
    .await;
    status(server.get(&path, b.token.expose()).query(&query(&b)), 403).await;
    // JSON remains strict: a string generation is not a Version number.
    status(
        server.post(
            &format!("{path}/cancel"),
            a.token.expose(),
            &serde_json::json!({ "instanceId": instance, "generation": generation.to_string() }),
        ),
        400,
    )
    .await;
    let after: NodeObservationResponse =
        json(server.get(&path, a.token.expose()).query(&query(&a)), 200).await;
    assert_eq!(after.task.id, created.task.id);
    assert_eq!(
        after.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    server.stop().await;
}

#[tokio::test]
async fn node_selectors_are_current_network_scoped_and_shared_subscriptions_cancel_independently() {
    // A: authorization probes are not a 30 s subscription-expiry test. Renew
    // at the advertised window before obtaining the final report configuration.
    let server = Server::new().await;
    let user = server.user("node-observer@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    let relay = server.enroll(&user, network.id, "relay").await;
    let foreign = server.network(&user).await;
    let outsider = server.enroll(&user, foreign.id, "outsider").await;
    server
        .configure(&user, network.id, |cfg| {
            cfg.dns.domain = "mesh.test".into();
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
            cfg.access = vec![meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                ip: vec!["tcp:443".into()],
                bidirectional: false,
            }];
            cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                via: vec![format!("node:{}", relay.id)],
            }];
        })
        .await;
    let first = demand(&a, "b.mesh.test");
    let mut initial: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        201,
    )
    .await;
    assert_eq!(initial.target.node_id, b.id);
    assert_eq!(initial.target.hostname, "b");
    assert_eq!(initial.task.target_node_id, initial.target.node_id);
    assert!(initial.task.latest.is_none());
    let source = slice(server.target(&a).await);
    assert!(source.permits.is_empty());
    assert_eq!(
        source.diagnostics[&initial.task.id].paths[0].nodes,
        vec![a.id, relay.id, b.id]
    );
    assert_eq!(source.peers[&b.id].addresses, initial.target.addresses);
    let retried: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        201,
    )
    .await;
    assert_eq!(retried.task.id, initial.task.id);
    assert_eq!(
        retried.task.subscriber_expires_at,
        initial.task.subscriber_expires_at
    );
    for target in [
        NodeObservationTarget::Name {
            name: "outsider".into(),
        },
        NodeObservationTarget::Name {
            name: "missing".into(),
        },
        NodeObservationTarget::MeshIp {
            address: "8.8.8.8".into(),
        },
    ] {
        keep_node_alive(&server, &a, &mut initial.task).await;
        status(
            server.post(
                "/agent-api/v1/observations",
                a.token.expose(),
                &CreateNodeObservation {
                    target,
                    subscription_id: id(),
                    ..first.clone()
                },
            ),
            404,
        )
        .await;
    }
    let wrong_instance = CreateNodeObservation {
        instance: b.instance.clone(),
        subscription_id: id(),
        ..first.clone()
    };
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &wrong_instance,
        ),
        401,
    )
    .await;
    status(
        server.post("/agent-api/v1/observations", user.token.expose(), &first),
        401,
    )
    .await;
    keep_node_alive(&server, &a, &mut initial.task).await;
    status(
        server
            .get(&selected(&first), b.token.expose())
            .query(&query(&b)),
        403,
    )
    .await;
    status(
        server
            .get(&selected(&first), outsider.token.expose())
            .query(&query(&outsider)),
        403,
    )
    .await;
    status(
        server
            .get(&selected(&first), a.token.expose())
            .query(&query(&b)),
        401,
    )
    .await;
    status(
        server.post(
            &format!("{}/cancel", selected(&first)),
            b.token.expose(),
            &b.instance,
        ),
        403,
    )
    .await;
    status(
        server.post(
            &format!("{}/renew", selected(&first)),
            outsider.token.expose(),
            &RenewNodeObservation {
                instance: outsider.instance.clone(),
                expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 20_000,
            },
        ),
        403,
    )
    .await;
    keep_node_alive(&server, &a, &mut initial.task).await;
    let icmp = CreateNodeObservation {
        subscription_id: id(),
        service: ObservationServiceScope::IcmpEcho,
        ..first.clone()
    };
    status(
        server.post("/agent-api/v1/observations", a.token.expose(), &icmp),
        403,
    )
    .await;
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &CreateNodeObservation {
                subscription_id: id(),
                selection: ObservationPath::Explicit {
                    path: vec![a.id, b.id],
                },
                ..first.clone()
            },
        ),
        403,
    )
    .await;
    keep_node_alive(&server, &a, &mut initial.task).await;
    let ip = initial.target.addresses.ipv4.unwrap().to_string();
    let second = CreateNodeObservation {
        subscription_id: id(),
        target: NodeObservationTarget::MeshIp { address: ip },
        ..first.clone()
    };
    let mut ip_view: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &second),
        201,
    )
    .await;
    assert_eq!(ip_view.task.id, initial.task.id);
    let user_request = request(&a, &b);
    let user_endpoint = format!("/api/v1/networks/{}/observations", network.id);
    let mut user_task: ObservationTask = json(
        server.post(&user_endpoint, user.token.expose(), &user_request),
        201,
    )
    .await;
    assert_eq!(user_task.id, initial.task.id);
    status(
        server
            .get(
                &format!(
                    "/agent-api/v1/observations/{}",
                    user_request.subscription_id
                ),
                a.token.expose(),
            )
            .query(&query(&a)),
        404,
    )
    .await;
    status(
        server.post(
            &format!(
                "/agent-api/v1/observations/{}/cancel",
                user_request.subscription_id
            ),
            a.token.expose(),
            &a.instance,
        ),
        403,
    )
    .await;
    let user_list: ObservationList =
        json(server.get(&user_endpoint, user.token.expose()), 200).await;
    assert_eq!(user_list.items.len(), 1);
    keep_node_alive(&server, &a, &mut initial.task).await;
    keep_node_alive(&server, &a, &mut ip_view.task).await;
    user_task = keep_alive(&server, &user, &user_endpoint, user_task).await;
    server.target(&a).await;
    let executor: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    let task = &executor.items[0];
    let now = chrono::Utc::now();
    let measurement = id();
    let channel = ProbeChannel {
        link_id: Digest::from_bytes([1; 32]),
        socket_id: Digest::from_bytes([2; 32]),
        peer_node_id: relay.id,
        transport: meshway_transport_spec::TransportProtocol::NativeUdp,
        local_address: "192.0.2.1:23456".parse().unwrap(),
        remote_address: "192.0.2.2:23456".parse().unwrap(),
        acquisition: LinkAcquisition::PublishedEndpoint,
    };
    // Explicitly a typed self-reported timeout. This is acceptance coverage,
    // not proof that a physical probe or successful packet flow occurred.
    let result = ObservationResult {
        instance_id: a.instance.instance_id,
        configuration: task.configuration.clone(),
        result: ObservationUpdate::Sample {
            path: vec![a.id, relay.id, b.id],
            path_segment: Sequence::new(1).unwrap(),
            sample: Box::new(ObservationSample {
                measurement_id: measurement,
                version: Version::MIN,
                window_start: now - chrono::Duration::milliseconds(1),
                window_end: now,
                partial: false,
                probe: Some(ProbeObservation {
                    channels: Some(ProbeChannels {
                        request: channel,
                        reply: None,
                    }),
                    measurement_id: measurement,
                    kind: ProbeKind::Path,
                    link_id: None,
                    transport: None,
                    path: vec![a.id, relay.id, b.id],
                    publication_id: task.configuration.execution_publication,
                    started_at: now - chrono::Duration::milliseconds(1),
                    completed_at: Some(now),
                    outcome: ProbeOutcome::Timeout,
                    duration_us: None,
                }),
                traffic: None,
                probe_counts: None,
            }),
        },
    };
    let result_path = format!("/agent-api/v1/tasks/{}/results", task.id);
    let mut wrong_peer = result.clone();
    if let ObservationUpdate::Sample { sample, .. } = &mut wrong_peer.result {
        sample
            .probe
            .as_mut()
            .unwrap()
            .channels
            .as_mut()
            .unwrap()
            .request
            .peer_node_id = b.id;
    }
    status(
        server.post(&result_path, a.token.expose(), &wrong_peer),
        400,
    )
    .await;
    let accepted: ObservationTask =
        json(server.post(&result_path, a.token.expose(), &result), 200).await;
    assert_eq!(accepted.phase, ObservationPhase::Running);
    assert_eq!(
        accepted.latest.unwrap().probe.unwrap().outcome,
        ProbeOutcome::Timeout
    );
    keep_alive(&server, &user, &user_endpoint, user_task.clone()).await;
    for request in [&first, &second] {
        let cancelled: NodeObservationCancelled = json(
            server.post(
                &format!("{}/cancel", selected(request)),
                a.token.expose(),
                &a.instance,
            ),
            200,
        )
        .await;
        let shared = cancelled.task.unwrap();
        assert_eq!(shared.id, user_task.id);
        assert_eq!(shared.subscription_id, None);
        assert_eq!(shared.subscriber_expires_at, None);
    }
    let user_list: ObservationList =
        json(server.get(&user_endpoint, user.token.expose()), 200).await;
    assert_eq!(user_list.items.len(), 1);
    status(
        server.post(
            &format!("{user_endpoint}/{}/cancel", user_request.subscription_id),
            user.token.expose(),
            &(),
        ),
        204,
    )
    .await;
    let cancelled: NodeObservationCancelled = json(
        server.post(
            &format!("{}/cancel", selected(&first)),
            a.token.expose(),
            &a.instance,
        ),
        200,
    )
    .await;
    assert!(cancelled.task.is_none());
    assert!(slice(server.target(&a).await).diagnostics.is_empty());
    // The same selector now names a different Node. The old ID must not create
    // another lifetime or reinterpret its original binding after revision loss.
    server
        .configure(&user, network.id, |cfg| {
            cfg.nodes.get_mut(&b.id).unwrap().hostname = "old-b".into();
            cfg.nodes.get_mut(&relay.id).unwrap().hostname = "b".into();
        })
        .await;
    status(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        404,
    )
    .await;
    server.stop().await;
}
