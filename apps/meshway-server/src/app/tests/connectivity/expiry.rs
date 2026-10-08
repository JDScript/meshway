use super::*;

#[tokio::test]
async fn original_absolute_expiry_and_gc_never_issue_a_replacement_nonce() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let mut req = request(&app, &a, &b).await;
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 - 1;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 60_000;
    assert_eq!(
        app.connectivity()
            .prepare(&a.token, req.clone())
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::NotAvailable
        }
    );
    req.expires_at_ms = chrono::Utc::now().timestamp_millis() as u64 + 500;
    let first = offer(&app, &a, req.clone()).await;
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!();
    };
    assert!(attempts.as_slice().is_empty());
    assert_eq!(
        app.connectivity().prepare(&a.token, req).await.unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::AttemptExpired
        }
    );
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_peer_session_deadline_recheck_is_terminal_even_if_storage_later_recovers() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let first = offer(&app, &a, request(&app, &a, &b).await).await;
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    // The SQL owner shortens the real peer-session deadline after prepare.
    let node = b.id;
    let old = app
        .executor
        .handle()
        .execute(move |connection| {
            let old: i64 = connection
                .query_row(
                    "SELECT max(expires_at) FROM node_sessions WHERE node_id=?1",
                    [node.as_uuid().as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![
                        node.as_uuid().as_bytes().as_slice(),
                        chrono::Utc::now().timestamp_millis() + 100
                    ],
                )
                .unwrap();
            old
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&a.token, first.attempt_id, a.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.executor
        .handle()
        .execute(move |connection| {
            connection
                .execute(
                    "UPDATE node_sessions SET expires_at=?2 WHERE node_id=?1",
                    rusqlite::params![node.as_uuid().as_bytes().as_slice(), old],
                )
                .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        app.connectivity()
            .accept(&b.token, first.attempt_id, b.cas(&first))
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable { .. }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_restart_changes_epoch_and_old_credentials_and_attempts_cannot_reappear() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b, _) = setup(true).await;
    let req = request(&app, &a, &b).await;
    let old = offer(&app, &a, req.clone()).await;
    let keys = meshway_protocol::node::NodeKeyMetadata {
        control: a.target.configuration.self_keys.control_key.clone(),
        dataplane: a.target.configuration.self_keys.dataplane_key.clone(),
    };
    app.shutdown().await.unwrap();
    let app = App::open(crate::Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert!(matches!(
        app.connectivity().query(&a.token, a.query()).await,
        Err(crate::error::Error::InvalidCredentials)
    ));
    let signer = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let token = session(&app.execution(), network, a.id, &keys, &signer).await;
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
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    let configuration = lease.target.unwrap();
    let QueryUdpTraversalResponse::Current {
        state_epoch,
        attempts,
        ..
    } = app
        .connectivity()
        .query(
            token.token.expose(),
            QueryUdpTraversalRequest {
                instance: instance.clone(),
                configuration: configuration.clone(),
            },
        )
        .await
        .unwrap()
    else {
        panic!();
    };
    assert_ne!(state_epoch, old.state_epoch);
    assert!(attempts.as_slice().is_empty());
    let mut req = req;
    req.instance = instance.clone();
    req.configuration = configuration.clone();
    assert_eq!(
        app.connectivity()
            .prepare(token.token.expose(), req)
            .await
            .unwrap(),
        PrepareUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    assert_eq!(
        app.connectivity()
            .accept(
                token.token.expose(),
                old.attempt_id,
                UdpTraversalAttemptRequest {
                    instance,
                    configuration,
                    state_epoch: old.state_epoch,
                    version: old.version,
                    digest: old.digest
                }
            )
            .await
            .unwrap(),
        AcceptUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn per_node_slots_are_bounded_across_distinct_authenticated_peer_pairs() {
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
    let mut peers = vec![(b.id, b.token, b.instance, b.report.sequence)];
    for number in 0..8u8 {
        let signer = ed25519_dalek::SigningKey::from_bytes(&[100 + number; 32]);
        let enrolled = super::super::node::enroll_tag_node(
            &app,
            owner.token.expose(),
            network,
            &signer,
            &format!("peer-{number}"),
            [120 + number; 32],
        )
        .await;
        let token = session(
            &app.execution(),
            network,
            enrolled.node_id,
            &enrolled.keys,
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
        peers.push((
            enrolled.node_id,
            token.token.expose().into(),
            InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            Sequence::MIN,
        ));
    }
    for (peer, _, _, _) in &peers {
        let decision = app
            .execution()
            .demand(
                &a.token,
                DemandRequest {
                    instance: a.instance.clone(),
                    request_id: rid(),
                    target: BusinessTarget::Mesh { node: *peer },
                    protocol: BusinessProtocol::Udp,
                    port: Some(53.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 60,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            decision.decision,
            DemandDecision::Permitted { .. }
        ));
    }
    // Finish the real publication/receipt work before producing observations.
    // Each producer owns its report sequence; a full network State projection
    // is unnecessary here and would age earlier candidates during setup.
    let mut synchronized = Vec::new();
    for (id, token, instance, sequence) in
        std::iter::once((a.id, a.token, a.instance, a.report.sequence)).chain(peers)
    {
        let target = sync_execution(&app, &token, &instance).await;
        synchronized.push((id, token, instance, sequence, target));
    }
    let mut ready = Vec::new();
    for (index, (id, token, instance, sequence, target)) in synchronized.into_iter().enumerate() {
        let mut report = report(
            &target,
            &instance,
            &format!("198.51.100.{}:23456", index + 1),
        );
        report.sequence = sequence;
        let mut node = Node {
            id,
            token,
            instance,
            target,
            report,
        };
        node.publish(&app).await;
        ready.push(node);
    }
    let a = ready.remove(0);
    // The batch shares one actually queried epoch. Every prepare still checks
    // its exact instance/configuration and both candidate facts transactionally.
    let QueryUdpTraversalResponse::Current { state_epoch, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current metadata");
    };
    let mut offers = Vec::new();
    for (index, peer) in ready.iter().enumerate() {
        let request = request_in_epoch(&a, peer, state_epoch);
        let result = app.connectivity().prepare(&a.token, request).await.unwrap();
        if index < 8 {
            let PrepareUdpTraversalResponse::Offer { offer } = result else {
                panic!(
                    "slot {index}: {result:?}; source report age={}, peer report age={}",
                    chrono::Utc::now() - a.report.observed_at,
                    chrono::Utc::now() - peer.report.observed_at,
                );
            };
            offers.push(offer);
        } else {
            assert_eq!(
                result,
                PrepareUdpTraversalResponse::Unavailable {
                    reason: UdpTraversalUnavailableReason::Capacity
                }
            );
        }
    }
    assert_eq!(offers.len(), 8);
    let QueryUdpTraversalResponse::Current { attempts, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current attempts");
    };
    assert_eq!(attempts.as_slice().len(), 8);
    for attempt in attempts.as_slice() {
        assert_eq!(attempt.phase, UdpTraversalPhase::Waiting);
        assert!(offers.contains(&attempt.offer));
    }
    let first = &offers[0];
    assert_eq!(
        app.connectivity()
            .cancel(&a.token, first.attempt_id, a.cas(first))
            .await
            .unwrap(),
        CancelUdpTraversalResponse::Cancelled,
    );
    let last = ready.last().unwrap();
    let _replacement = offer(&app, &a, request_in_epoch(&a, last, state_epoch)).await;
    // Cancellation freed the slot: none of the original fixed deadlines has
    // elapsed, including through the current-authority query above.
    let now = chrono::Utc::now().timestamp_millis() as u64;
    assert!(offers.iter().all(|offer| offer.expires_at_ms > now));
    app.shutdown().await.unwrap();
}
