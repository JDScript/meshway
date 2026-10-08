use super::*;

#[tokio::test]
async fn enrollment_key_idempotency_tag_relogin_and_permanent_delete() {
    use meshway_protocol::{configuration::ConfigurationRequest as Edit, enrollment::*};
    use sha2::{Digest as _, Sha256};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let owner = app
        .accounts()
        .register(
            "lifecycle@example.com".into(),
            Secret::new("a proper long owner password".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(owner.token.expose(), "lifecycle".into(), rid())
        .await
        .unwrap()
        .id;
    let initial = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            Edit::Operations {
                expected_revision: Revision::MIN,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceTagsOwners {
                    tags_owners: BTreeMap::from([("tag:server".into(), vec![])]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let creation = CreateEnrollmentKeyRequest {
        request_id: rid(),
        tags: vec!["tag:server".into()],
        initial_settings: InitialNodeSettings {
            hostname: Some("server".into()),
            listeners: BTreeMap::new(),
        },
        expires_at_ms: store::execution::now().unwrap() as u64 + 60_000,
    };
    let issued = app
        .enrollment()
        .create_key(owner.token.expose(), network, creation.clone())
        .await
        .unwrap();
    let repeated = app
        .enrollment()
        .create_key(owner.token.expose(), network, creation)
        .await
        .unwrap();
    assert_eq!(issued.key.id, repeated.key.id);
    assert!(repeated.secret.is_none());
    assert_eq!(repeated.secret_delivery, SecretDelivery::Unavailable);
    let secret = issued.secret.unwrap();
    let signer = SigningKey::from_bytes(&[31; 32]);
    let verifier = "v".repeat(43);
    let make_intent = |existing, hostname: &str| EnrollmentIntent {
        id: EnrollmentRequestId::new(random_uuid().unwrap()).unwrap(),
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [32; 32].into(),
        },
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: hostname.into(),
        existing,
    };
    let intent = make_intent(None, "reported-host");
    let digest = proof::request_digest(&intent).unwrap();
    let key_request = || KeyEnrollmentRequest {
        enrollment_key: WireSecret::new(secret.expose().into()),
        verifier: WireSecret::new(verifier.clone()),
        request: CreateEnrollmentRequest {
            intent: intent.clone(),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    signer
                        .sign(&enrollment_transcript(intent.id, digest, "create"))
                        .to_bytes(),
                ),
            ),
        },
    };
    let enrolled = app.enrollment().redeem_key(key_request()).await.unwrap();
    let recovered = app.enrollment().redeem_key(key_request()).await.unwrap();
    assert_eq!(enrolled, recovered);
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(snapshot.revision > initial.revision);
    let node = &snapshot.document.configuration.nodes[&enrolled.node_id];
    assert_eq!(node.hostname, "server");
    assert!(node.listeners.is_empty());
    assert!(matches!(node.identity, NodeIdentity::Tag { .. }));
    assert!(!snapshot.bindings.node_users.contains_key(&enrolled.node_id));
    let tag_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let login = make_intent(
        Some(ExistingNodeBinding {
            network_id: network,
            node_id: enrolled.node_id,
            key_id: enrolled.keys.control.id,
            key_generation: enrolled.keys.control.generation,
        }),
        "not-a-config-edit",
    );
    let login_digest = proof::request_digest(&login).unwrap();
    app.enrollment()
        .create_request(CreateEnrollmentRequest {
            intent: login.clone(),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    signer
                        .sign(&enrollment_transcript(login.id, login_digest, "create"))
                        .to_bytes(),
                ),
            ),
        })
        .await
        .unwrap();
    app.enrollment()
        .approve(
            owner.token.expose(),
            login.id,
            ApproveEnrollmentRequest {
                network_id: network,
            },
        )
        .await
        .unwrap();
    let device_proof = |operation| EnrollmentDeviceProof {
        verifier: WireSecret::new(verifier.clone()),
        signature: WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                signer
                    .sign(&enrollment_transcript(login.id, login_digest, operation))
                    .to_bytes(),
            ),
        ),
    };
    let pending = app
        .enrollment()
        .poll(login.id, device_proof("poll"))
        .await
        .unwrap();
    let restored = app
        .enrollment()
        .redeem(
            login.id,
            RedeemEnrollmentRequest {
                proof: device_proof("redeem"),
                code: pending.code.unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.node_id, enrolled.node_id);
    assert_eq!(restored.keys, enrolled.keys);
    assert!(matches!(
        app.execution()
            .authenticate(tag_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    let user_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    // A second login with the same User identity changes no configuration but
    // still ends the previous control session and its instance.
    let stale_instance = app
        .execution()
        .acquire(
            user_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let before_relogin = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let again = make_intent(
        Some(ExistingNodeBinding {
            network_id: network,
            node_id: enrolled.node_id,
            key_id: enrolled.keys.control.id,
            key_generation: enrolled.keys.control.generation,
        }),
        "same-identity",
    );
    let again_digest = proof::request_digest(&again).unwrap();
    let again_proof = |operation| EnrollmentDeviceProof {
        verifier: WireSecret::new(verifier.clone()),
        signature: WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                signer
                    .sign(&enrollment_transcript(again.id, again_digest, operation))
                    .to_bytes(),
            ),
        ),
    };
    app.enrollment()
        .create_request(CreateEnrollmentRequest {
            intent: again.clone(),
            signature: again_proof("create").signature,
        })
        .await
        .unwrap();
    app.enrollment()
        .approve(
            owner.token.expose(),
            again.id,
            ApproveEnrollmentRequest {
                network_id: network,
            },
        )
        .await
        .unwrap();
    let pending = app
        .enrollment()
        .poll(again.id, again_proof("poll"))
        .await
        .unwrap();
    let relogged = app
        .enrollment()
        .redeem(
            again.id,
            RedeemEnrollmentRequest {
                proof: again_proof("redeem"),
                code: pending.code.unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(relogged.node_id, enrolled.node_id);
    assert_eq!(relogged.keys, enrolled.keys);
    assert!(matches!(
        app.execution()
            .authenticate(user_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    let user_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    assert!(matches!(
        app.execution()
            .renew(
                user_session.token.expose(),
                InstanceRequest {
                    instance_id: stale_instance.instance_id,
                    generation: stale_instance.generation,
                },
            )
            .await,
        Err(Error::BindingConflict)
    ));
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(current.revision, before_relogin.revision);
    let restored = &current.document.configuration.nodes[&enrolled.node_id];
    assert!(current.bindings.users.is_empty());
    assert_eq!(
        current.bindings.node_users[&enrolled.node_id].user_id,
        owner.session.user.id
    );
    assert_eq!(restored.hostname, "server");
    assert!(matches!(&restored.identity,NodeIdentity::User{user} if user=="lifecycle@example.com"));
    let lease = app
        .execution()
        .acquire(
            user_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let instance = InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    };
    let target = lease.target.unwrap();
    let mut receipt = ReceiptRequest {
        instance: instance.clone(),
        attempt: Version::MIN,
        sequence: Sequence::MIN,
        target: target.clone(),
        phase: ReceiptPhase::Saved,
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure: None,
    };
    app.execution()
        .receipt(user_session.token.expose(), receipt.clone())
        .await
        .unwrap();
    receipt.phase = ReceiptPhase::Applied;
    receipt.sequence = Sequence::new(2).unwrap();
    app.execution()
        .receipt(user_session.token.expose(), receipt)
        .await
        .unwrap();
    let progress = app
        .execution()
        .progress(owner.token.expose(), network, target.execution_publication)
        .await
        .unwrap();
    assert!(progress.nodes[&enrolled.node_id].applied);
    let node = enrolled.node_id;
    app.executor.handle().execute(move|c|c.execute("UPDATE node_execution SET execution_expires_at=?1 WHERE network_id=?2 AND node_id=?3",rusqlite::params![store::execution::now().unwrap(),network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()]).unwrap()).await.unwrap();
    let expired = app
        .execution()
        .progress(owner.token.expose(), network, target.execution_publication)
        .await
        .unwrap();
    assert!(!expired.nodes[&node].applied);
    assert_eq!(expired.nodes[&node].freshness, ReceiptFreshness::Expired);
    assert!(expired.nodes[&node].last_received_at_ms.is_some());
    let renewed = app
        .execution()
        .renew(user_session.token.expose(), instance)
        .await
        .unwrap();
    let renewed = renewed.target.unwrap();
    assert_ne!(renewed.execution_publication, target.execution_publication);
    let progress = app
        .execution()
        .progress(owner.token.expose(), network, renewed.execution_publication)
        .await
        .unwrap();
    assert!(!progress.nodes[&node].applied);
    let deleted = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            Edit::Operations {
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
    assert_eq!(deleted.deleted_node_ids, vec![enrolled.node_id]);
    assert!(app.state_runtime.lock().unwrap().hub.events.after(0,network).unwrap().iter().any(|entry| matches!(entry.change, crate::effect::Effect::NodeRetired { node_id, .. } if node_id == enrolled.node_id)));
    assert!(matches!(
        app.execution()
            .authenticate(user_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    assert!(matches!(
        app.enrollment().redeem_key(key_request()).await,
        Err(Error::InvalidCredentials)
    ));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
