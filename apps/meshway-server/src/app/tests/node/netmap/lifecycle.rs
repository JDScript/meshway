//! Real membership/enrollment transactions and the stored netmaps they change.
use super::*;

#[tokio::test]
async fn member_removal_atomically_withdraws_user_netmap_and_preserves_tag_nodes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, tag, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let member = app
        .accounts()
        .register(
            "member@example.com".into(),
            Secret::new("a sufficiently long password".into()),
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
    let (_, membership) = app
        .networks()
        .redeem_join_link(member.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let signer = SigningKey::from_bytes(&[41; 32]);
    let enrolled = enroll_user_node(&app, member.token.expose(), network, &signer, [51; 32]).await;
    let token = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let netmap = app
        .netmaps()
        .sync(
            token.token.expose(),
            meshway_protocol::SyncRequest {
                netmap: None,
                state: None,
                server_key_generation: None,
                traversal_requests: Default::default(),
                associations: None,
                lease: None,
            },
        )
        .await
        .unwrap();
    let impact = app
        .networks()
        .removal_impact(member.token.expose(), network, None, rid())
        .await
        .unwrap();
    assert_eq!(impact.node_ids, [enrolled.node_id]);
    app.executor.handle().execute(|db| db.execute_batch("CREATE TRIGGER fail_member_removal BEFORE UPDATE OF revoked_at ON memberships BEGIN SELECT RAISE(ABORT,'injected failure'); END;")).await.unwrap().unwrap();
    assert!(
        app.networks()
            .remove_member(
                member.token.expose(),
                network,
                None,
                impact.version,
                impact.revision,
                rid()
            )
            .await
            .is_err()
    );
    let still_current = app
        .netmaps()
        .sync(
            token.token.expose(),
            meshway_protocol::SyncRequest {
                netmap: None,
                state: None,
                server_key_generation: None,
                traversal_requests: Default::default(),
                associations: None,
                lease: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(still_current.lease.netmap, netmap.lease.netmap);
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_member_removal"))
        .await
        .unwrap()
        .unwrap();
    let revision = app
        .networks()
        .remove_member(
            member.token.expose(),
            network,
            Some(membership),
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert!(revision > impact.revision);
    assert!(matches!(
        app.netmaps()
            .sync(
                token.token.expose(),
                meshway_protocol::SyncRequest {
                    netmap: None,
                    state: None,
                    server_key_generation: None,
                    traversal_requests: Default::default(),
                    associations: None,
                    lease: None,
                }
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    let reader = app.netmaps.clone();
    app.executor
        .handle()
        .read(move |tx| {
            assert!(reader.current(tx, network, enrolled.node_id)?.is_none());
            assert!(reader.current(tx, network, tag)?.is_some());
            Ok(())
        })
        .await
        .unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn enrollment_key_replay_and_permanent_deletion_follow_the_same_netmap_identity() {
    use meshway_protocol::enrollment::*;
    use sha2::{Digest as _, Sha256};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, _, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let creation = CreateEnrollmentKeyRequest {
        request_id: rid(),
        tags: vec!["tag:test".into()],
        initial_settings: InitialNodeSettings {
            hostname: Some("key-node".into()),
            listeners: BTreeMap::new(),
        },
        expires_at_ms: store::execution::now().unwrap() as u64 + 60_000,
    };
    let issued = app
        .enrollment()
        .create_key(owner.token.expose(), network, creation.clone())
        .await
        .unwrap();
    let retried = app
        .enrollment()
        .create_key(owner.token.expose(), network, creation)
        .await
        .unwrap();
    assert_eq!(issued.key.id, retried.key.id);
    assert!(retried.secret.is_none());
    let signer = SigningKey::from_bytes(&[31; 32]);
    let verifier = "v".repeat(43);
    let intent = EnrollmentIntent {
        id: EnrollmentRequestId::new(random_uuid().unwrap()).unwrap(),
        existing: None,
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [32; 32].into(),
        },
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: "reported-host".into(),
    };
    let signature = signer.sign(&enrollment_transcript(
        intent.id,
        proof::request_digest(&intent).unwrap(),
        "create",
    ));
    let secret = issued.secret.unwrap();
    let request = || KeyEnrollmentRequest {
        enrollment_key: WireSecret::new(secret.expose().to_owned()),
        verifier: WireSecret::new(verifier.clone()),
        request: CreateEnrollmentRequest {
            intent: intent.clone(),
            signature: WireSecret::new(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
        },
    };
    let enrolled = app.enrollment().redeem_key(request()).await.unwrap();
    assert_eq!(
        app.enrollment().redeem_key(request()).await.unwrap(),
        enrolled
    );
    let token = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let response = app
        .netmaps()
        .sync(
            token.token.expose(),
            meshway_protocol::SyncRequest {
                netmap: None,
                state: None,
                server_key_generation: None,
                traversal_requests: Default::default(),
                associations: None,
                lease: None,
            },
        )
        .await
        .unwrap();
    let meshway_protocol::SyncNetmap::Full { netmap } = response.netmap else {
        panic!("full netmap");
    };
    assert_eq!(netmap.netmap.node_id, enrolled.node_id);
    assert_eq!(netmap.netmap.self_.hostname, "key-node");
    assert!(matches!(
        netmap.netmap.self_.identity,
        NodeIdentity::Tag { .. }
    ));
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::DeleteNode {
                    node_id: enrolled.node_id,
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(matches!(
        app.enrollment().redeem_key(request()).await,
        Err(Error::InvalidCredentials)
    ));
    assert!(matches!(
        app.execution().authenticate(token.token.expose()).await,
        Err(Error::InvalidCredentials)
    ));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
