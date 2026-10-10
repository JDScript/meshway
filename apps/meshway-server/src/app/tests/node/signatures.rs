use super::*;

#[tokio::test]
async fn signed_authentication_replay_rotation_and_controller_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let service = app.execution();
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge = service
        .challenge(service.signed_challenge_fixture(
            NodeChallengeRequest {
                network_id: network,
                node_id: a,
                key_id: keys.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&intent).unwrap(),
            },
            &signer,
        ))
        .await
        .unwrap();
    let signature = URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(&challenge)).to_bytes());
    let request = || NodeSessionRequest {
        intent: intent.clone(),
        proof: NodeProof {
            challenge_id: challenge.id,
            signature: WireSecret::new(signature.clone()),
        },
    };
    let issued = service.session(request()).await.unwrap();
    assert!(matches!(
        service.session(request()).await,
        Err(Error::InvalidCredentials)
    ));
    assert_eq!(
        service
            .authenticate(issued.token.expose())
            .await
            .unwrap()
            .node_id(),
        a
    );
    let mut other = challenge.clone();
    other.network_id = NetworkId::new(random_uuid().unwrap()).unwrap();
    assert!(proof::verify(&keys.control.public_key, &other, &signature).is_err());
    let new = SigningKey::from_bytes(&[9; 32]);
    let intent = NodeRotationIntent {
        request_id: rid(),
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: new.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [19; 32].into(),
        },
    };
    let challenge = service
        .challenge(service.signed_challenge_fixture(
            NodeChallengeRequest {
                network_id: network,
                node_id: a,
                key_id: keys.control.id,
                operation: ProofOperation::Rotate,
                request_digest: proof::request_digest(&intent).unwrap(),
            },
            &signer,
        ))
        .await
        .unwrap();
    let rotated = service
        .rotate(NodeRotationRequest {
            intent,
            old_proof: NodeProof {
                challenge_id: challenge.id,
                signature: WireSecret::new(
                    URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
                ),
            },
            new_signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(new.sign(&proof_transcript(&challenge)).to_bytes()),
            ),
        })
        .await
        .unwrap();
    assert_eq!(rotated.control.generation.get(), 2);
    assert!(matches!(
        service.authenticate(issued.token.expose()).await,
        Err(Error::InvalidCredentials)
    ));
    let fresh = session(&service, network, a, &rotated, &new).await;
    let original_instance = service
        .acquire(
            fresh.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert!(matches!(
        app.execution().authenticate(fresh.token.expose()).await,
        Err(Error::InvalidCredentials)
    ));
    let renewed_session = session(&app.execution(), network, a, &rotated, &new).await;
    assert_eq!(
        app.execution()
            .acquire(
                renewed_session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: original_instance.instance_id,
                    capabilities: caps(),
                }
            )
            .await,
        Err(Error::BindingConflict)
    );
    let resumed = app
        .execution()
        .acquire(
            renewed_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    assert!(resumed.generation > original_instance.generation);
    // An unchanged immutable target may be reused; its old process cannot be.
    assert_ne!(resumed.instance_id, original_instance.instance_id);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn review_released_instance_id_cannot_be_reacquired() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, ak, _, signer, _) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &signer).await;
    let request = AcquireInstanceRequest {
        instance_id: iid(),
        capabilities: caps(),
    };
    let ai = s.acquire(at.token.expose(), request.clone()).await.unwrap();
    assert_eq!(
        s.acquire(at.token.expose(), request.clone()).await.unwrap(),
        ai
    );
    s.release(
        at.token.expose(),
        InstanceRequest {
            instance_id: ai.instance_id,
            generation: ai.generation,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        s.acquire(at.token.expose(), request).await,
        Err(Error::BindingConflict)
    );
    app.shutdown().await.unwrap();
}
