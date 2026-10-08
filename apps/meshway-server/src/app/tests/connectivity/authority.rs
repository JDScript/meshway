use super::*;

#[tokio::test]
async fn bilateral_readiness_replay_identity_and_cancel_are_real_current_authority() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    assert!(app.stun_ready());
    let QueryUdpTraversalResponse::Current {
        stun_servers,
        attempts,
        ..
    } = app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(stun_servers.as_slice().is_empty());
    assert!(attempts.as_slice().is_empty());
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
    assert!(
        !serde_json::to_string(report)
            .unwrap()
            .contains("203.0.113.53")
    );
    let request = request(&app, &a, &b).await;
    let first = offer(&app, &a, request.clone()).await;
    assert_eq!(offer(&app, &a, request.clone()).await, first);
    let mut changed = request.clone();
    changed.expires_at_ms -= 1;
    assert_eq!(
        app.connectivity().prepare(&a.token, changed).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::RequestChanged
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&b.token, b.query()).await.unwrap()
    else {
        panic!();
    };
    assert_eq!(attempts.as_slice()[0].phase, UdpTraversalPhase::Waiting);
    // A new telemetry sequence, with the original candidate, is not a new mapping.
    a.report.state.system.hostname = "telemetry-update".into();
    a.publish(&app).await;
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Active { .. }
    ));
    assert_eq!(offer(&app, &a, request.clone()).await, first);
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert_eq!(attempts.as_slice()[0].phase, UdpTraversalPhase::Active);
    for _ in 0..2 {
        assert_eq!(
            app.connectivity()
                .cancel(&b.token, first.attempt_id, b.cas(&first))
                .await
                .unwrap(),
            CancelUdpTraversalResponse::Cancelled
        );
    }
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    assert!(matches!(
        app.connectivity().prepare(&a.token, request).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn candidate_cas_physical_resource_and_scope_races_are_closed_outcomes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, mut a, b, _) = setup(true).await;
    let original = request(&app, &a, &b).await;
    let encoded = serde_json::to_value(&original).unwrap();
    for (pointer, value) in [
        (
            "/localCandidate/observationDigest",
            serde_json::json!("ff".repeat(32)),
        ),
        (
            "/localCandidate/socket/socketGeneration",
            serde_json::json!(2),
        ),
        (
            "/peerCandidate/socket/instance/generation",
            serde_json::json!(2),
        ),
        (
            "/peerCandidate/socket/resource/key",
            serde_json::json!("unrelated-listener"),
        ),
        (
            "/carrier/listenerKey",
            serde_json::json!("unrelated-listener"),
        ),
        ("/instance/generation", serde_json::json!(2)),
        ("/configuration/digest", serde_json::json!("fe".repeat(32))),
    ] {
        let mut value_json = encoded.clone();
        *value_json.pointer_mut(pointer).unwrap() = value;
        let bad: PrepareUdpTraversalRequest = serde_json::from_value(value_json).unwrap();
        assert!(
            matches!(
                app.connectivity().prepare(&a.token, bad).await.unwrap(),
                PrepareUdpTraversalResponse::Unavailable { .. }
            ),
            "{pointer}"
        );
    }
    let first = offer(&app, &a, original.clone()).await;
    let mut bad_cas = a.cas(&first);
    bad_cas.digest = [99; 32].into();
    assert_eq!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, bad_cas)
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptChanged
        }
    );
    let mut another = original.clone();
    another.request_id = rid();
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, another.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::Capacity
        }
    );
    // Generation changes preserve the report's other facts but invalidate the offer.
    a.report.state.sockets.get_mut("socket").unwrap().generation = Sequence::new(2).unwrap();
    a.publish(&app).await;
    assert_eq!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::CandidateChanged
        }
    );
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&b.token, b.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(attempts.as_slice().is_empty());
    another.local_candidate = a.candidate();
    let next = offer(&app, &a, another).await;
    assert_ne!(next.nonce, first.nonce);
    assert_ne!(next.attempt_id, first.attempt_id);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn withdrawal_between_offer_and_accept_never_installs_an_active_grant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, demand) = setup(true).await;
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    app.connectivity()
        .accept(&a.token, first.attempt_id, a.cas(&first))
        .await
        .unwrap();
    app.execution()
        .cancel_demand(
            &a.token,
            CancelDemandRequest {
                instance: a.instance.clone(),
                request_id: demand,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    assert!(matches!(
        app.connectivity().query(&a.token, a.query()).await.unwrap(),
        QueryUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn metadata_is_available_without_business_but_candidate_reports_do_not_grant_punching() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(false).await;
    let request = request(&app, &a, &b).await;
    assert_eq!(
        app.connectivity().prepare(&a.token, request).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn foreign_network_participants_cannot_prepare_accept_or_cancel_an_offer() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(owner.token.expose(), "foreign traversal".into(), rid())
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: network.current_revision,
                request_id: rid(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: BTreeMap::from([("tag:test".into(), vec![])]),
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    let signer = ed25519_dalek::SigningKey::from_bytes(&[90; 32]);
    let foreign = super::super::node::enroll_tag_node(
        &app,
        owner.token.expose(),
        network.id,
        &signer,
        "foreign",
        [91; 32],
    )
    .await;
    let token = session(
        &app.execution(),
        network.id,
        foreign.node_id,
        &foreign.keys,
        &signer,
    )
    .await;
    let lease = app
        .execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let foreign = sync(
        &app,
        foreign.node_id,
        token.token.expose().into(),
        InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        "198.51.100.9:23456",
    )
    .await;
    assert!(matches!(
        app.connectivity()
            .prepare(&a.token, request(&app, &a, &foreign).await)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable { .. }
    ));
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    assert_eq!(
        app.connectivity()
            .accept(&foreign.token, first.attempt_id, foreign.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    assert_eq!(
        app.connectivity()
            .cancel(&foreign.token, first.attempt_id, foreign.cas(&first))
            .await
            .unwrap(),
        CancelUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Active { .. }
    ));
    assert!(matches!(
        app.connectivity()
            .query(owner.token.expose(), a.query())
            .await,
        Err(crate::error::Error::InvalidCredentials)
    ));
    app.shutdown().await.unwrap();
}
