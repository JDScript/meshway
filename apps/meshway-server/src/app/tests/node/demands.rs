use super::*;

#[tokio::test]
async fn demand_retry_is_not_blocked_by_an_expired_sibling_demand() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, _, signer_a, _) = fixture().await;
    let service = app.execution();
    let a_session = session(&service, network, a, &ak, &signer_a).await;
    let ai = service
        .acquire(
            a_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let instance = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    let demand = |request_id, port: u16, duration_seconds| DemandRequest {
        instance: instance.clone(),
        request_id,
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Tcp,
        port: Some(port.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds,
    };
    let short = demand(rid(), 443, 1);
    let long = demand(rid(), 8443, 300);
    service
        .demand(a_session.token.expose(), short.clone())
        .await
        .unwrap();
    let first = service
        .demand(a_session.token.expose(), long.clone())
        .await
        .unwrap();
    // The short demand expires without any renew or configuration read in
    // between; the long one is still valid and its retry must recover.
    let short_id = short.request_id;
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE execution_demands SET expires_at=0 WHERE id=?1",
                [short_id.as_uuid().as_bytes().as_slice()],
            )
            .unwrap()
        })
        .await
        .unwrap();
    let retried = service
        .demand(a_session.token.expose(), long)
        .await
        .unwrap();
    assert_eq!(retried.decision, first.decision);
    assert_ne!(retried.publication, first.publication);
    assert_eq!(
        service.demand(a_session.token.expose(), short).await,
        Err(Error::BindingConflict)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn multi_demand_saved_applied_fencing_and_revocation() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &asigner).await;
    let bt = session(&s, network, b, &bk, &bsigner).await;
    let ai = s
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let bi = s
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let ar = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    let br = InstanceRequest {
        instance_id: bi.instance_id,
        generation: bi.generation,
    };
    let demand = |request_id, target| DemandRequest {
        instance: ar.clone(),
        request_id,
        target: BusinessTarget::Mesh { node: target },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 60,
    };
    let one = rid();
    let two = rid();
    s.demand(at.token.expose(), demand(one, b)).await.unwrap();
    s.demand(at.token.expose(), demand(two, b)).await.unwrap();
    let target = s
        .configuration(
            at.token.expose(),
            NodeConfigurationRequest {
                instance: ar.clone(),
                base: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(target.full.as_ref().unwrap().configuration.permits.len(), 2);
    let target = s
        .renew(at.token.expose(), ar.clone())
        .await
        .unwrap()
        .target
        .unwrap();
    let mut receipt = ReceiptRequest {
        instance: ar.clone(),
        attempt: Version::MIN,
        sequence: Sequence::MIN,
        target: target.clone(),
        phase: ReceiptPhase::Applied,
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure: None,
    };
    assert_eq!(
        s.receipt(at.token.expose(), receipt.clone()).await,
        Err(Error::ReceiptPrecondition)
    );
    receipt.phase = ReceiptPhase::Saved;
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    receipt.phase = ReceiptPhase::Applied;
    receipt.sequence = Sequence::new(2).unwrap();
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    s.receipt(at.token.expose(), receipt).await.unwrap();
    s.cancel_demand(
        at.token.expose(),
        CancelDemandRequest {
            instance: ar.clone(),
            request_id: one,
        },
    )
    .await
    .unwrap();
    let target = s
        .configuration(
            bt.token.expose(),
            NodeConfigurationRequest {
                instance: br,
                base: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(target.full.unwrap().configuration.permits.len(), 1);
    s.acquire(
        at.token.expose(),
        AcquireInstanceRequest {
            instance_id: iid(),
            capabilities: caps(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        s.renew(at.token.expose(), ar).await,
        Err(Error::BindingConflict)
    );
    app.executor
        .handle()
        .execute(move |c| {
            c.execute(
                "UPDATE nodes SET credentials_revoked_at=?1 WHERE id=?2",
                rusqlite::params![
                    store::execution::now().unwrap(),
                    a.as_uuid().as_bytes().as_slice()
                ],
            )
            .unwrap();
        })
        .await
        .unwrap();
    assert!(matches!(
        s.authenticate(at.token.expose()).await,
        Err(Error::InvalidCredentials)
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn independent_demands_keep_unrelated_publication_lease_and_progress() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let csigner = SigningKey::from_bytes(&[61; 32]);
    let dsigner = SigningKey::from_bytes(&[62; 32]);
    let c = enroll_tag_node(&app, owner.token.expose(), network, &csigner, "c", [71; 32]).await;
    let d = enroll_tag_node(&app, owner.token.expose(), network, &dsigner, "d", [72; 32]).await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.defaults.topology = TopologyDefault::Deny;
    config.topology = [(a, b), (c.node_id, d.node_id)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            transports: Input::Omitted,
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
            keepalive: false,
        })
        .collect();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    let s = app.execution();
    let mut sessions = Vec::new();
    let mut instances = Vec::new();
    for (node, keys, signer) in [
        (a, &ak, &asigner),
        (b, &bk, &bsigner),
        (c.node_id, &c.keys, &csigner),
        (d.node_id, &d.keys, &dsigner),
    ] {
        let session = session(&s, network, node, keys, signer).await;
        let lease = s
            .acquire(
                session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: iid(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
        sessions.push(session);
    }
    let demand = |instance: InstanceRequest, target| DemandRequest {
        instance,
        request_id: rid(),
        target: BusinessTarget::Mesh { node: target },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    let cd = demand(instances[2].clone(), d.node_id);
    let first = s
        .demand(sessions[2].token.expose(), cd.clone())
        .await
        .unwrap();
    let peer = s
        .renew(sessions[3].token.expose(), instances[3].clone())
        .await
        .unwrap();
    let mut receipt = ReceiptRequest {
        instance: instances[3].clone(),
        target: peer.target.clone().unwrap(),
        phase: ReceiptPhase::Saved,
        attempt: Version::MIN,
        sequence: Sequence::MIN,
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure: None,
    };
    s.receipt(sessions[3].token.expose(), receipt.clone())
        .await
        .unwrap();
    receipt.phase = ReceiptPhase::Applied;
    receipt.sequence = Sequence::new(1).unwrap();
    s.receipt(sessions[3].token.expose(), receipt)
        .await
        .unwrap();
    let ab = demand(instances[0].clone(), b);
    s.demand(sessions[0].token.expose(), ab.clone())
        .await
        .unwrap();
    assert_eq!(
        s.demand(sessions[2].token.expose(), cd).await.unwrap(),
        first
    );
    let after_retry = s
        .configuration(
            sessions[3].token.expose(),
            NodeConfigurationRequest {
                instance: instances[3].clone(),
                base: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(after_retry.lease, peer);
    s.cancel_demand(
        sessions[0].token.expose(),
        CancelDemandRequest {
            instance: instances[0].clone(),
            request_id: ab.request_id,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        s.demand(sessions[0].token.expose(), ab).await,
        Err(Error::BindingConflict)
    );
    s.revoke_key(
        owner.token.expose(),
        network,
        b,
        RevokeNodeKeyRequest {
            key_id: bk.control.id,
            expected_generation: bk.control.generation,
        },
    )
    .await
    .unwrap();
    let after = s
        .configuration(
            sessions[3].token.expose(),
            NodeConfigurationRequest {
                instance: instances[3].clone(),
                base: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(after.lease, peer);
    let progress = s
        .progress(owner.token.expose(), network, first.publication)
        .await
        .unwrap();
    assert_eq!(progress.withdrawn_at_ms, None);
    assert!(progress.nodes[&d.node_id].applied);
    assert!(progress.nodes[&d.node_id].current);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
