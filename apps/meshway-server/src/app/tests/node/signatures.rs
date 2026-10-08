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

#[tokio::test]
async fn review_rotation_http_response_loss_recovers_same_keys() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let old_session = session(&app.execution(), network, a, &keys, &signer).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let new = SigningKey::from_bytes(&[29; 32]);
    let intent = NodeRotationIntent {
        request_id: rid(),
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: new.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [39; 32].into(),
        },
    };
    let challenge = app
        .execution()
        .challenge(app.execution().signed_challenge_fixture(
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
    let request = NodeRotationRequest {
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
    };
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let url = format!("http://{address}/agent-api/v1/auth/rotate");
    // Discard the successful body: the device has no newly issued key IDs.
    let lost = client.post(&url).json(&request).send().await.unwrap();
    assert!(lost.status().is_success(), "{}", lost.status());
    drop(lost);
    let response = client.post(&url).json(&request).send().await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let recovered: NodeKeyMetadata = response.json().await.unwrap();
    assert_eq!(recovered.control.generation.get(), 2);
    let again: NodeKeyMetadata = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(again, recovered);
    let new_session = session(&app.execution(), network, a, &recovered, &new).await;
    let discovery = || NodeDnsQueryRequest {
        name: "unknown.mesh.example".into(),
    };
    assert!(matches!(
        app.execution()
            .query_dns(old_session.token.expose(), discovery(), rid())
            .await,
        Err(Error::InvalidCredentials)
    ));
    assert!(
        app.execution()
            .query_dns(new_session.token.expose(), discovery(), rid())
            .await
            .is_ok()
    );
    let mut changed: NodeRotationRequest =
        serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
    changed.intent.request_id = rid();
    assert_eq!(
        client
            .post(&url)
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let mut forged: NodeRotationRequest =
        serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
    forged.new_signature = WireSecret::new(
        URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
    );
    assert_eq!(
        client
            .post(&url)
            .json(&forged)
            .send()
            .await
            .unwrap()
            .status(),
        401
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
    // Revoking the retired generation records the revocation but leaves the
    // execution established with the current generation untouched; repeating
    // it reports the key as already revoked.
    let current = session(&app.execution(), network, a, &recovered, &new).await;
    let lease = app
        .execution()
        .acquire(
            current.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    assert!(lease.target.is_some());
    let retired = || RevokeNodeKeyRequest {
        key_id: keys.control.id,
        expected_generation: keys.control.generation,
    };
    app.execution()
        .revoke_key(owner.token.expose(), network, a, retired())
        .await
        .unwrap();
    assert!(matches!(
        app.execution()
            .revoke_key(owner.token.expose(), network, a, retired())
            .await,
        Err(Error::StateConflict)
    ));
    let renewed = app
        .execution()
        .renew(
            current.token.expose(),
            InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(renewed.target, lease.target);
    app.execution()
        .revoke_key(
            owner.token.expose(),
            network,
            a,
            RevokeNodeKeyRequest {
                key_id: recovered.control.id,
                expected_generation: recovered.control.generation,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        app.execution()
            .renew(
                current.token.expose(),
                InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    assert_eq!(
        client
            .post(&url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    server.abort();
    let _ = server.await;
    app.shutdown().await.unwrap();
}
