use super::*;

#[tokio::test]
async fn node_absolute_renewal_does_not_slide_on_retry_and_restart_or_revocation_does_not_restore_it()
 {
    // C: the Node subscription's renew_after (10 s) must pass in real time;
    // replaying an absolute renewal must not slide the confirmed deadline.
    let server = Server::new().await;
    let user = server.user("node-lease@example.com").await;
    let network = server.network(&user).await;
    let mut a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
            cfg.access = vec![meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                ip: vec!["icmp:*".into()],
                bidirectional: false,
            }];
        })
        .await;
    let first = CreateNodeObservation {
        service: ObservationServiceScope::IcmpEcho,
        ..demand(&a, "b")
    };
    let created: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        201,
    )
    .await;
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &demand(&a, "b"),
        ),
        403,
    )
    .await;
    let invalid = RenewNodeObservation {
        instance: a.instance.clone(),
        // A 31 s client deadline becomes legal after 1 s of HTTP scheduling.
        // Exercise the invalid upper bound without racing that transport delay.
        expires_at_ms: (chrono::Utc::now() + chrono::Duration::days(1)).timestamp_millis() as u64,
    };
    status(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &invalid,
        ),
        400,
    )
    .await;
    wait_until(created.task.renew_after).await;
    server.target(&a).await;
    let read: NodeObservationResponse = json(
        server
            .get(&selected(&first), a.token.expose())
            .query(&query(&a)),
        200,
    )
    .await;
    assert_eq!(
        read.task.subscriber_expires_at,
        created.task.subscriber_expires_at
    );
    let renewal = RenewNodeObservation {
        instance: a.instance.clone(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 29_000,
    };
    let renewed: NodeObservationResponse = json(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &renewal,
        ),
        200,
    )
    .await;
    assert_pre_expiry_budget(&created.task);
    assert_eq!(
        renewed
            .task
            .subscriber_expires_at
            .unwrap()
            .timestamp_millis() as u64,
        renewal.expires_at_ms
    );
    let replay: NodeObservationResponse = json(
        server.post(
            &format!("{}/renew", selected(&first)),
            a.token.expose(),
            &renewal,
        ),
        200,
    )
    .await;
    assert_eq!(
        replay.task.subscriber_expires_at,
        renewed.task.subscriber_expires_at
    );
    assert_eq!(replay.task.renew_after, renewed.task.renew_after);
    let server = server.restart().await;
    status(
        server
            .get(&selected(&first), a.token.expose())
            .query(&query(&a)),
        401,
    )
    .await;
    server.resume(&mut a, network.id).await;
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &CreateNodeObservation {
                instance: a.instance.clone(),
                ..first.clone()
            },
        ),
        409,
    )
    .await;
    status(
        server.post("/agent-api/v1/observations", a.token.expose(), &first),
        401,
    )
    .await;
    let fresh = CreateNodeObservation {
        instance: a.instance.clone(),
        subscription_id: id(),
        ..first.clone()
    };
    let _: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &fresh),
        201,
    )
    .await;
    status(
        server.post(
            &format!("/api/v1/networks/{}/nodes/{}/keys/revoke", network.id, a.id),
            user.token.expose(),
            &meshway_protocol::RevokeNodeKeyRequest {
                key_id: a.keys.control.id,
                expected_generation: a.keys.control.generation,
            },
        ),
        204,
    )
    .await;
    status(
        server
            .get(&selected(&fresh), a.token.expose())
            .query(&query(&a)),
        401,
    )
    .await;
    server.stop().await;
}

#[tokio::test]
async fn every_required_path_role_must_support_diagnostics_before_publication() {
    let server = Server::new().await;
    let user = server.user("node-capability@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    let relay = server.enroll(&user, network.id, "relay").await;
    server
        .configure(&user, network.id, |cfg| {
            for node in cfg.nodes.values_mut() {
                node.listeners =
                    serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#)
                        .unwrap();
            }
            cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                via: vec![format!("node:{}", relay.id)],
            }];
            cfg.access = vec![meshway_configuration::AccessRule {
                remark: String::new(),
                src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)],
                ip: vec!["tcp:443".into()],
                bidirectional: false,
            }];
        })
        .await;
    let mut source_instance = a.instance.clone();
    let mut instances = std::collections::BTreeMap::from([
        (a.id, a.instance.clone()),
        (relay.id, relay.instance.clone()),
        (b.id, b.instance.clone()),
    ]);
    for node in [&a, &relay, &b] {
        let mut capabilities = caps();
        capabilities.remove("feature:path-observation");
        let lease: InstanceLease = json(
            server.post(
                "/agent-api/v1/instances",
                node.token.expose(),
                &AcquireInstanceRequest {
                    instance_id: id(),
                    capabilities,
                },
            ),
            200,
        )
        .await;
        instances.insert(
            node.id,
            meshway_protocol::InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
        );
        if node.id == a.id {
            source_instance = meshway_protocol::InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            };
        }
        let request = CreateNodeObservation {
            instance: source_instance.clone(),
            ..demand(&a, "b")
        };
        let unsupported: NodeObservationResponse = json(
            server.post("/agent-api/v1/observations", a.token.expose(), &request),
            201,
        )
        .await;
        assert_eq!(unsupported.task.phase, ObservationPhase::Unsupported);
        assert_eq!(unsupported.task.unsupported_metrics, request.metrics);
        assert!(unsupported.task.path.is_none() && unsupported.task.latest.is_none());
        assert_eq!(unsupported.target.node_id, b.id);
        let read: NodeObservationResponse = json(
            server
                .get(&selected(&request), a.token.expose())
                .query(&NodeObservationQuery {
                    instance_id: source_instance.instance_id,
                    generation: source_instance.generation,
                }),
            200,
        )
        .await;
        assert_eq!(read.task.phase, ObservationPhase::Unsupported);
        let retry: NodeObservationResponse = json(
            server.post("/agent-api/v1/observations", a.token.expose(), &request),
            201,
        )
        .await;
        assert_eq!(retry.task.id, unsupported.task.id);
        assert_eq!(
            retry.task.subscriber_expires_at,
            unsupported.task.subscriber_expires_at
        );
        let user_endpoint = format!("/api/v1/networks/{}/observations", network.id);
        let user_unsupported: ObservationTask = json(
            server.post(
                &user_endpoint,
                user.token.expose(),
                &super::super::request(&a, &b),
            ),
            201,
        )
        .await;
        assert_eq!(user_unsupported.phase, ObservationPhase::Unsupported);
        assert_eq!(user_unsupported.unsupported_metrics, request.metrics);
        for service in [
            ObservationServiceScope::IcmpEcho,
            ObservationServiceScope::Tcp {
                port: 80.try_into().unwrap(),
            },
        ] {
            status(
                server.post(
                    "/agent-api/v1/observations",
                    a.token.expose(),
                    &CreateNodeObservation {
                        subscription_id: id(),
                        service: service.clone(),
                        ..request.clone()
                    },
                ),
                403,
            )
            .await;
            status(
                server.post(
                    &user_endpoint,
                    user.token.expose(),
                    &CreateObservation {
                        service,
                        ..super::super::request(&a, &b)
                    },
                ),
                403,
            )
            .await;
        }
        status(
            server.post(
                "/agent-api/v1/observations",
                a.token.expose(),
                &CreateNodeObservation {
                    subscription_id: id(),
                    selection: ObservationPath::Explicit {
                        path: vec![a.id, b.id],
                    },
                    ..request.clone()
                },
            ),
            403,
        )
        .await;
        let list: ObservationList =
            json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
        assert!(list.items.is_empty());
        for participant in [&a, &relay, &b] {
            let configuration: NodeConfigurationResponse = json(
                server.post(
                    "/agent-api/v1/configuration/query",
                    participant.token.expose(),
                    &NodeConfigurationRequest {
                        instance: instances[&participant.id].clone(),
                        base: None,
                    },
                ),
                200,
            )
            .await;
            assert!(slice(configuration).diagnostics.is_empty());
        }
        let restored: InstanceLease = json(
            server.post(
                "/agent-api/v1/instances",
                node.token.expose(),
                &AcquireInstanceRequest {
                    instance_id: id(),
                    capabilities: caps(),
                },
            ),
            200,
        )
        .await;
        instances.insert(
            node.id,
            meshway_protocol::InstanceRequest {
                instance_id: restored.instance_id,
                generation: restored.generation,
            },
        );
        if node.id == a.id {
            source_instance = meshway_protocol::InstanceRequest {
                instance_id: restored.instance_id,
                generation: restored.generation,
            };
        }
    }
    let request = CreateNodeObservation {
        instance: source_instance,
        ..demand(&a, "b")
    };
    let accepted: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &request),
        201,
    )
    .await;
    assert!(accepted.task.latest.is_none());
    server.stop().await;
}

#[tokio::test]
async fn capable_authorized_candidate_survives_an_extra_unsupported_candidate_without_bypassing_waypoints()
 {
    let server = Server::new().await;
    let user = server.user("node-candidates@example.com").await;
    let network = server.network(&user).await;
    let a = server.enroll(&user, network.id, "a").await;
    let b = server.enroll(&user, network.id, "b").await;
    let relay = server.enroll(&user, network.id, "required").await;
    let extra = server.enroll(&user, network.id, "extra").await;
    let mut capabilities = caps();
    capabilities.remove("feature:path-observation");
    let extra_lease: InstanceLease = json(
        server.post(
            "/agent-api/v1/instances",
            extra.token.expose(),
            &AcquireInstanceRequest {
                instance_id: id(),
                capabilities,
            },
        ),
        200,
    )
    .await;
    server
        .configure(&user, network.id, |cfg| {
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
    let auto = demand(&a, "b");
    let accepted: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &auto),
        201,
    )
    .await;
    assert_eq!(accepted.task.phase, ObservationPhase::Preparing);
    assert_eq!(accepted.task.unsupported_metrics, vec![MetricKind::Traffic]);
    let source = slice(server.target(&a).await);
    assert!(source.permits.is_empty());
    assert_eq!(
        source.diagnostics[&accepted.task.id]
            .paths
            .iter()
            .map(|path| path.nodes.clone())
            .collect::<Vec<_>>(),
        vec![vec![a.id, relay.id, b.id]]
    );
    assert!(!source.peers.contains_key(&extra.id));
    let pinned = CreateNodeObservation {
        subscription_id: id(),
        selection: ObservationPath::Explicit {
            path: vec![a.id, extra.id, relay.id, b.id],
        },
        ..auto.clone()
    };
    let unsupported: NodeObservationResponse = json(
        server.post("/agent-api/v1/observations", a.token.expose(), &pinned),
        201,
    )
    .await;
    assert_eq!(unsupported.task.phase, ObservationPhase::Unsupported);
    assert_eq!(unsupported.task.unsupported_metrics, pinned.metrics);
    assert!(unsupported.task.path.is_none() && unsupported.task.latest.is_none());
    let source = slice(server.target(&a).await);
    assert!(source.diagnostics.contains_key(&accepted.task.id));
    assert!(!source.diagnostics.contains_key(&unsupported.task.id));
    let extra_target: NodeConfigurationResponse = json(
        server.post(
            "/agent-api/v1/configuration/query",
            extra.token.expose(),
            &NodeConfigurationRequest {
                instance: meshway_protocol::InstanceRequest {
                    instance_id: extra_lease.instance_id,
                    generation: extra_lease.generation,
                },
                base: None,
            },
        ),
        200,
    )
    .await;
    assert!(slice(extra_target).diagnostics.is_empty());
    status(
        server.post(
            "/agent-api/v1/observations",
            a.token.expose(),
            &CreateNodeObservation {
                subscription_id: id(),
                selection: ObservationPath::Explicit {
                    path: vec![a.id, b.id],
                },
                ..auto.clone()
            },
        ),
        403,
    )
    .await;
    let mut capabilities = caps();
    capabilities.remove("feature:path-observation");
    let _: InstanceLease = json(
        server.post(
            "/agent-api/v1/instances",
            relay.token.expose(),
            &AcquireInstanceRequest {
                instance_id: id(),
                capabilities,
            },
        ),
        200,
    )
    .await;
    let withdrawn: NodeObservationResponse = json(
        server
            .get(&selected(&auto), a.token.expose())
            .query(&query(&a)),
        200,
    )
    .await;
    assert_eq!(withdrawn.task.phase, ObservationPhase::Unsupported);
    assert_eq!(withdrawn.task.unsupported_metrics, auto.metrics);
    assert!(withdrawn.task.path.is_none() && withdrawn.task.latest.is_none());
    assert!(slice(server.target(&a).await).diagnostics.is_empty());
    let tasks: ObservationList =
        json(server.get("/agent-api/v1/tasks", a.token.expose()), 200).await;
    assert!(tasks.items.is_empty());
    server.stop().await;
}
