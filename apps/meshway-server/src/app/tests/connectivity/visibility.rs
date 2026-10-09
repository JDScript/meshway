use super::*;

#[tokio::test]
async fn diagnostic_only_publication_requires_common_live_adjacency_before_disclosure_and_offer() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(false).await;
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    assert!(!view.nodes.contains_key(&b.id));
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    app.observations()
        .create_observation(
            ReadIdentity::user_session(owner.token.expose(), network).unwrap(),
            meshway_protocol::observations::CreateObservation {
                service: meshway_configuration::execution::ObservationServiceScope::Tcp {
                    port: 443.try_into().unwrap(),
                },
                subscription_id: meshway_model::ObservationSubscriptionId::new(
                    random_uuid().unwrap(),
                )
                .unwrap(),
                source_node_id: a.id,
                target_node_id: b.id,
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a.id, b.id],
                },
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            },
        )
        .await
        .unwrap();
    let a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    // Peer has not acquired the matching diagnostic publication yet.
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    assert!(a.target.configuration.permits.is_empty());
    assert!(!a.target.configuration.diagnostics.is_empty());
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&b.id].report else {
        panic!();
    };
    assert_eq!(report.endpoints[0].candidate_reference, b.candidate());
    let request = request(&app, &a, &b).await;
    let now = chrono::Utc::now().timestamp_millis() as u64;
    let id = *a.target.configuration.diagnostics.keys().next().unwrap();
    for case in 0..5 {
        let mut peer = b.target.configuration.clone();
        match case {
            0 => {
                peer.diagnostics.clear();
            }
            1 => {
                peer.diagnostics.get_mut(&id).unwrap().instance_id =
                    InstanceId::new(random_uuid().unwrap()).unwrap();
            }
            2 => {
                peer.diagnostics.get_mut(&id).unwrap().service =
                    meshway_configuration::execution::ObservationServiceScope::IcmpEcho;
            }
            3 => {
                peer.diagnostics.get_mut(&id).unwrap().expires_at_ms = now;
            }
            _ => {
                peer.diagnostics.get_mut(&id).unwrap().paths[0]
                    .dial_candidates
                    .clear();
            }
        }
        assert!(
            crate::execution::connectivity::common_witness(
                &a.target.configuration,
                &peer,
                &request.carrier,
                now,
                None,
                |_| panic!("mismatched bilateral grant reached full-path validation"),
            )
            .unwrap()
            .is_none()
        );
    }
    let offer = offer(&app, &a, request).await;
    assert_eq!(
        offer.local.source,
        meshway_model::observations::EndpointKind::Stun
    );
    assert_eq!(
        offer.peer.source,
        meshway_model::observations::EndpointKind::Stun
    );
    assert!(
        !serde_json::to_string(&offer)
            .unwrap()
            .contains("203.0.113.53")
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, offer.attempt_id, a.cas(&offer))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, offer.attempt_id, b.cas(&offer))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Active { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn complete_logical_path_visibility_does_not_disclose_nonadjacent_endpoints() {
    use meshway_configuration::{TopologyAction, TopologyDefault, TopologyRule};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(false).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
    let relay = super::super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        network,
        &signer,
        "relay",
        [78; 32],
    )
    .await;
    let rt = session(
        &app.execution(),
        network,
        relay.node_id,
        &relay.keys,
        &signer,
    )
    .await;
    let lease = app
        .execution()
        .acquire(
            rt.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    cfg.defaults.topology = TopologyDefault::Deny;
    cfg.topology = [(a.id, relay.node_id), (relay.node_id, b.id)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            transports: meshway_configuration::Input::Omitted,
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
 keepalive: false,
        })
        .collect();
    cfg.routing.mesh = vec![meshway_configuration::MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", a.id)],
        dst: vec![format!("node:{}", b.id)],
        via: vec![format!("node:{}", relay.node_id)],
    }];
    cfg.access = vec![meshway_configuration::AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", a.id)],
        dst: vec![format!("node:{}", b.id)],
        ip: vec!["udp:53".into()],
        bidirectional: false,
    }];
    // Apply through the real configuration owner, not a fabricated execution DTO.
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: meshway_protocol::WireSecret::new(
                    meshway_configuration::canonical_json(&cfg)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let response = app
        .execution()
        .demand(
            &a.token,
            DemandRequest {
                instance: a.instance.clone(),
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b.id },
                protocol: BusinessProtocol::Udp,
                port: Some(53.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    let DemandDecision::Permitted { permit } = response.decision else {
        panic!();
    };
    assert!(
        permit
            .paths
            .iter()
            .all(|path| path.nodes == vec![a.id, relay.node_id, b.id])
    );
    let a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    let b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    let relay = sync(
        &app,
        relay.node_id,
        rt.token.expose().into(),
        InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        "198.51.100.3:23456",
    )
    .await;
    let view = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: a.token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    for (peer, count) in [(&b, 0), (&relay, 1)] {
        let Some(VisibleReport::Peer { report }) = &view.nodes[&peer.id].report else {
            panic!();
        };
        assert_eq!(report.endpoints.len(), count);
    }
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let _adjacent = offer(&app, &a, request(&app, &a, &relay).await).await;
    let full = app
        .user_state_service()
        .unwrap()
        .query(
            ReadIdentity::user_session(owner.token.expose(), network).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(&full.nodes[&b.id].report, Some(VisibleReport::Full { report }) if !report.sockets.is_empty())
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn dialer_can_use_a_different_owned_listener_but_listener_side_is_exact() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, _) = setup(true).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut cfg = current.document.configuration.clone();
    for node in [a.id, b.id] {
        cfg.nodes.get_mut(&node).unwrap().listeners.insert(
            "local-source".into(),
            serde_json::from_value(serde_json::json!({"type":"native_udp","listenPort":23457}))
                .unwrap(),
        );
    }
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: meshway_protocol::WireSecret::new(
                    meshway_configuration::canonical_json(&cfg)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let mut a = sync(&app, a.id, a.token, a.instance, "198.51.100.1:23456").await;
    let mut b = sync(&app, b.id, b.token, b.instance, "198.51.100.2:23456").await;
    let use_other_listener = |node: &mut Node| {
        let listener = node.report.state.listeners.remove("udp").unwrap();
        node.report
            .state
            .listeners
            .insert("local-source".into(), listener);
        let socket = node.report.state.sockets.get_mut("socket").unwrap();
        socket.resource = ResourceRef::Listener {
            key: "local-source".into(),
        };
        socket.bound_address.set_port(23457);
        let mut address: std::net::SocketAddr = socket.endpoints[0].address.parse().unwrap();
        address.set_port(23457);
        socket.endpoints[0].address = address.to_string();
    };
    use_other_listener(&mut a);
    a.publish(&app).await;
    let req = request(&app, &a, &b).await;
    assert_eq!(req.carrier.listener_key, "udp");
    let first = offer(&app, &a, req).await;
    app.connectivity()
        .cancel(&a.token, first.attempt_id, a.cas(&first))
        .await
        .unwrap();
    use_other_listener(&mut b);
    b.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn original_candidate_expiry_and_declared_listener_bind_are_checked_at_prepare() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    a.report
        .state
        .sockets
        .get_mut("socket")
        .unwrap()
        .bound_address
        .set_port(23457);
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    let socket = a.report.state.sockets.get_mut("socket").unwrap();
    socket.bound_address.set_port(23456);
    socket.endpoints[0].observed_at = chrono::Utc::now() - chrono::Duration::seconds(3);
    socket.endpoints[0].expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &b).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    app.shutdown().await.unwrap();
}
