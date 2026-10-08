use super::*;

#[tokio::test]
async fn node_sse_frames_reauthorize_without_report_history_for_multiple_subscribers() {
    use crate::state::{
        service::ReadIdentity,
        stream::{StreamPoll, StreamSource},
    };
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _b, keys, _bk, signer, _bs) = fixture().await;
    let execution = app.execution();
    let session = session(&execution, network, a, &keys, &signer).await;
    execution
        .acquire(
            session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let service = app.node_state_service().unwrap();
    let mut subscriptions = Vec::new();
    for _ in 0..4 {
        subscriptions.push(
            service
                .open_stream(
                    ReadIdentity::Node {
                        authorization: session.token.expose().to_owned(),
                    },
                    None,
                    rid(),
                )
                .await
                .unwrap(),
        );
    }
    let handle = app.executor.handle();
    handle
        .read(|_| Ok(crate::execution::Execution::take_state_work()))
        .await
        .unwrap();
    for subscription in &subscriptions {
        for _ in 0..2 {
            assert!(matches!(
                service.poll(subscription).await.unwrap(),
                StreamPoll::Frame(_)
            ));
        }
    }
    let counts = handle
        .read(|_| Ok(crate::execution::Execution::take_state_work()))
        .await
        .unwrap();
    assert_eq!(
        counts,
        (8, 0),
        "one real Node scope check per frame, no historical material reads"
    );
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    execution
        .revoke_key(
            owner.token.expose(),
            network,
            a,
            RevokeNodeKeyRequest {
                key_id: keys.control.id,
                expected_generation: keys.control.generation,
            },
        )
        .await
        .unwrap();
    for subscription in &subscriptions {
        let StreamPoll::Frame(frame) = service.poll(subscription).await.unwrap() else {
            panic!("expected explicit closure");
        };
        let event: meshway_protocol::NodeEvent = serde_json::from_str(&frame.data).unwrap();
        assert!(matches!(
            event.0,
            meshway_protocol::StreamEvent::Closed {
                reason: meshway_protocol::events::ClosedReason::SessionExpired
            }
        ));
    }
    let counts = handle
        .read(|_| Ok(crate::execution::Execution::take_state_work()))
        .await
        .unwrap();
    assert_eq!(
        counts,
        (4, 0),
        "queued snapshots cannot reuse pre-revocation authorization"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_configuration_progress_keeps_fixed_targets_and_orders_real_receipts() {
    use crate::state::service::ReadIdentity;
    use meshway_protocol::live::ApplicationPhase;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let execution = app.execution();
    let at = session(&execution, network, a, &ak, &signer_a).await;
    let bt = session(&execution, network, b, &bk, &signer_b).await;
    let ai = execution
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let bi = execution
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
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
    let mut input = NetworkConfigurationInput::from(&current.document.configuration);
    for node in [a, b] {
        input.nodes.get_mut(&node).unwrap().listeners =
            serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23457}}"#).unwrap();
    }
    let applied = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            },
            rid(),
        )
        .await
        .unwrap();
    let ai_request = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    let bi_request = InstanceRequest {
        instance_id: bi.instance_id,
        generation: bi.generation,
    };
    let a_target = execution
        .configuration(
            at.token.expose(),
            NodeConfigurationRequest {
                instance: ai_request.clone(),
                base: None,
            },
        )
        .await
        .unwrap()
        .lease
        .target
        .unwrap();
    let b_target = execution
        .configuration(
            bt.token.expose(),
            NodeConfigurationRequest {
                instance: bi_request,
                base: None,
            },
        )
        .await
        .unwrap()
        .lease
        .target
        .unwrap();
    let targets = BTreeMap::from([(a, a_target.clone()), (b, b_target)]);
    let revision = applied.revision;
    let state = app.user_state_service().unwrap();
    let read = |token: &str| {
        let identity = ReadIdentity::user_session(token, network).unwrap();
        let provider = Arc::clone(&state.provider);
        state.handle.execute(move |db| {
            let tx = db.transaction().unwrap();
            provider.progress(&tx, &identity, revision, chrono::Utc::now())
        })
    };
    let pending = read(owner.token.expose()).await.unwrap().unwrap();
    assert_eq!(pending.summary.target_count, 2);
    assert_eq!(pending.summary.pending, 2);
    assert_eq!(
        pending
            .nodes
            .iter()
            .map(|(node, progress)| (*node, progress.target.clone()))
            .collect::<BTreeMap<_, _>>(),
        targets
    );
    let receipt = |attempt, sequence, phase, failure| ReceiptRequest {
        instance: ai_request.clone(),
        target: a_target.clone(),
        phase,
        attempt: Version::new(attempt).unwrap(),
        sequence: Sequence::new(sequence).unwrap(),
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure,
    };
    execution
        .receipt(at.token.expose(), receipt(1, 0, ReceiptPhase::Saved, None))
        .await
        .unwrap();
    execution
        .renew(at.token.expose(), ai_request.clone())
        .await
        .unwrap();
    execution
        .receipt(
            at.token.expose(),
            receipt(1, 1, ReceiptPhase::Applied, None),
        )
        .await
        .unwrap();
    assert_eq!(
        read(owner.token.expose()).await.unwrap().unwrap().nodes[&a].phase,
        ApplicationPhase::Applied
    );
    execution
        .receipt(at.token.expose(), receipt(2, 0, ReceiptPhase::Saved, None))
        .await
        .unwrap();
    execution
        .receipt(
            at.token.expose(),
            receipt(1, 2, ReceiptPhase::Applied, None),
        )
        .await
        .unwrap();
    let latest = read(owner.token.expose()).await.unwrap().unwrap();
    assert_eq!(latest.nodes[&a].phase, ApplicationPhase::Saved);
    assert_eq!(latest.nodes[&a].attempt.unwrap().get(), 2);
    let failed = receipt(2, 1, ReceiptPhase::Failed, Some(ApplyFailure::Dns));
    execution
        .receipt(at.token.expose(), failed.clone())
        .await
        .unwrap();
    execution.receipt(at.token.expose(), failed).await.unwrap();
    let latest = read(owner.token.expose()).await.unwrap().unwrap();
    assert_eq!(latest.summary.failed, 1);
    assert_eq!(latest.summary.pending, 1);
    execution
        .demand(
            at.token.expose(),
            DemandRequest {
                instance: ai_request,
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    let superseded = read(owner.token.expose()).await.unwrap().unwrap();
    assert_eq!(superseded.summary.target_count, 2);
    assert_eq!(superseded.nodes[&a].phase, ApplicationPhase::Failed);
    assert_eq!(superseded.nodes[&a].freshness, ReceiptFreshness::Superseded);
    let member = app
        .accounts()
        .register(
            "progress-member@example.test".into(),
            Secret::new("valid long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let link = app
        .networks()
        .create_join_link(
            owner.token.expose(),
            network,
            MembershipRole::Member,
            600,
            rid(),
        )
        .await
        .unwrap();
    app.networks()
        .redeem_join_link(member.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let hidden = read(member.token.expose()).await.unwrap().unwrap();
    assert_eq!(hidden.summary.target_count, 0);
    assert!(hidden.nodes.is_empty());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn review_late_failed_receipt_does_not_override_newer_applied() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, keys, _, signer, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let s = app.execution();
    let at = session(&s, network, a, &keys, &signer).await;
    let lease = s
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let mut receipt = ReceiptRequest {
        instance: InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        target: lease.target.unwrap(),
        phase: ReceiptPhase::Saved,
        attempt: Version::MIN,
        sequence: Sequence::MIN,
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure: None,
    };
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    let mut late = receipt.clone();
    late.phase = ReceiptPhase::Failed;
    late.sequence = Sequence::new(2).unwrap();
    late.failure = Some(ApplyFailure::Tun);
    receipt.phase = ReceiptPhase::Applied;
    receipt.attempt = Version::new(2).unwrap();
    let mut saved_retry = receipt.clone();
    saved_retry.phase = ReceiptPhase::Saved;
    s.receipt(at.token.expose(), saved_retry).await.unwrap();
    receipt.sequence = Sequence::new(2).unwrap();
    receipt.reported_at_ms += 1;
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    let latest = app.state_runtime.lock().unwrap().hub.events.sequence();
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    s.receipt(at.token.expose(), late.clone()).await.unwrap();
    s.receipt(at.token.expose(), late.clone()).await.unwrap();
    late.failure = Some(ApplyFailure::Dns);
    assert_eq!(
        s.receipt(at.token.expose(), late).await,
        Err(Error::ReceiptContentConflict)
    );
    assert_eq!(
        app.state_runtime.lock().unwrap().hub.events.sequence(),
        latest
    );
    let progress = s
        .progress(
            owner.token.expose(),
            network,
            receipt.target.execution_publication,
        )
        .await
        .unwrap();
    assert!(progress.nodes[&a].applied);
    assert!(!progress.nodes[&a].failed);
    // Multiple saved reports are real ordered evidence, not phase-key collisions.
    receipt.phase = ReceiptPhase::Saved;
    receipt.attempt = Version::new(3).unwrap();
    receipt.sequence = Sequence::MIN;
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    receipt.sequence = Sequence::new(1).unwrap();
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    let progress = s
        .progress(
            owner.token.expose(),
            network,
            receipt.target.execution_publication,
        )
        .await
        .unwrap();
    assert!(!progress.nodes[&a].applied);
    receipt.phase = ReceiptPhase::Applied;
    receipt.sequence = Sequence::new(2).unwrap();
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    let mut invalid = receipt.clone();
    invalid.target.digest = [42; 32].into();
    assert_eq!(
        s.receipt(at.token.expose(), invalid).await,
        Err(Error::ReceiptTargetMismatch)
    );
    let mut invalid = receipt.clone();
    invalid.instance.instance_id = iid();
    assert_eq!(
        s.receipt(at.token.expose(), invalid).await,
        Err(Error::ReceiptInstanceMismatch)
    );
    let mut invalid = receipt.clone();
    invalid.instance.generation = Version::new(receipt.instance.generation.get() + 1).unwrap();
    assert_eq!(
        s.receipt(at.token.expose(), invalid).await,
        Err(Error::ReceiptInstanceMismatch)
    );
    let evidence: i64 = app
        .executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT count(*) FROM execution_receipts WHERE node_id=?1",
                [a.as_uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
    assert_eq!(evidence, 7);
    // A demand replaces this node's target. Evidence for the previous target
    // still enters history, while a target the node never held does not.
    let moved = s
        .demand(
            at.token.expose(),
            DemandRequest {
                instance: receipt.instance.clone(),
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert_ne!(moved.publication, receipt.target.execution_publication);
    let mut late = receipt.clone();
    late.phase = ReceiptPhase::Saved;
    late.attempt = Version::new(4).unwrap();
    late.sequence = Sequence::MIN;
    s.receipt(at.token.expose(), late.clone()).await.unwrap();
    late.phase = ReceiptPhase::Applied;
    late.sequence = Sequence::new(2).unwrap();
    s.receipt(at.token.expose(), late.clone()).await.unwrap();
    // History keeps the evidence (saved and the late applied) while the
    // superseded publication no longer counts as a current application.
    let progress = s
        .progress(
            owner.token.expose(),
            network,
            late.target.execution_publication,
        )
        .await
        .unwrap();
    assert!(!progress.nodes[&a].current);
    assert!(progress.nodes[&a].saved);
    assert!(!progress.nodes[&a].applied);
    assert_eq!(
        progress.nodes[&a].freshness,
        meshway_protocol::execution::ReceiptFreshness::Superseded
    );
    let late_rows: i64 = app
        .executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT count(*) FROM execution_receipts WHERE node_id=?1 AND attempt=4",
                [a.as_uuid().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
    assert_eq!(late_rows, 2);
    let mut never_held = late.clone();
    never_held.target.digest = [43; 32].into();
    assert_eq!(
        s.receipt(at.token.expose(), never_held).await,
        Err(Error::ReceiptTargetMismatch)
    );
    app.shutdown().await.unwrap();
}
