use super::*;

#[test]
fn node_and_management_configuration_openapi_components_are_distinct() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let schemas = &spec["components"]["schemas"];
    assert!(schemas["ConfigurationRequest"]["oneOf"].is_array());
    assert!(schemas["NodeConfigurationRequest"]["properties"]["instance"].is_object());
    assert!(schemas["ConfigurationResponse"]["properties"]["configuration"].is_object());
    assert!(schemas["NodeConfigurationResponse"]["properties"]["lease"].is_object());
    assert_eq!(
        spec["paths"]["/agent-api/v1/configuration/query"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/NodeConfigurationRequest"
    );
}

#[tokio::test]
async fn actual_http_signatures_and_pkce_polling() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    use meshway_protocol::enrollment::*;
    use sha2::{Digest as _, Sha256};
    let (_dir, app, network, node, _, keys, _, signer, _) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{address}");
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge: NodeChallenge = client
        .post(format!("{base}/agent-api/v1/auth/challenges"))
        .json(&app.execution().signed_challenge_fixture(
            NodeChallengeRequest {
                network_id: network,
                node_id: node,
                key_id: keys.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&intent).unwrap(),
            },
            &signer,
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let request = NodeSessionRequest {
        intent,
        proof: NodeProof {
            challenge_id: challenge.id,
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
            ),
        },
    };
    let result: NodeSessionResponse = client
        .post(format!("{base}/agent-api/v1/auth/session"))
        .json(&request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .post(format!("{base}/agent-api/v1/auth/session"))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let lease: InstanceLease = client
        .post(format!("{base}/agent-api/v1/instances"))
        .bearer_auth(result.token.expose())
        .json(&AcquireInstanceRequest {
            instance_id: iid(),
            capabilities: caps(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(lease.target.is_some());
    let user = app
        .accounts()
        .register(
            "approval@example.com".into(),
            Secret::new("another sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let own = app
        .networks()
        .create(user.token.expose(), "approved network".into(), rid())
        .await
        .unwrap();
    let key = SigningKey::from_bytes(&[11; 32]);
    let verifier = "a".repeat(43);
    let intent = EnrollmentIntent {
        existing: None,
        id: EnrollmentRequestId::new(random_uuid().unwrap()).unwrap(),
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: key.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [21; 32].into(),
        },
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: "new-device".into(),
    };
    let digest = proof::request_digest(&intent).unwrap();
    let id = intent.id;
    let request = CreateEnrollmentRequest {
        signature: WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                key.sign(&enrollment_transcript(id, digest, "create"))
                    .to_bytes(),
            ),
        ),
        intent,
    };
    client
        .post(format!("{base}/agent-api/v1/enrollment-requests"))
        .json(&request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .post(format!("{base}/api/v1/device-authorizations/{id}/approve"))
        .bearer_auth(user.token.expose())
        .json(&ApproveEnrollmentRequest { network_id: own.id })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let proof = |operation: &str| EnrollmentDeviceProof {
        verifier: WireSecret::new(verifier.clone()),
        signature: WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                key.sign(&enrollment_transcript(id, digest, operation))
                    .to_bytes(),
            ),
        ),
    };
    let poll: EnrollmentPollResponse = client
        .post(format!(
            "{base}/agent-api/v1/enrollment-requests/{id}/query"
        ))
        .json(&proof("poll"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(poll.request.phase, EnrollmentPhase::Approved);
    assert!(poll.code.is_some());
    assert!(poll.node_id.is_none());
    // A failing configuration version rolls the whole redemption back: the
    // request stays approved and redeemable.
    app.executor
        .handle()
        .execute(|db| {
            db.execute_batch("CREATE TRIGGER reject_version BEFORE INSERT ON configuration_versions BEGIN SELECT RAISE(ABORT, 'injected failure'); END;")
        })
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        app.enrollment()
            .redeem(
                id,
                RedeemEnrollmentRequest {
                    proof: proof("redeem"),
                    code: poll.code.unwrap()
                }
            )
            .await,
        Err(Error::Storage(_))
    ));
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER reject_version;"))
        .await
        .unwrap()
        .unwrap();
    let after = app.enrollment().poll(id, proof("poll")).await.unwrap();
    assert_eq!(after.request.phase, EnrollmentPhase::Approved);
    assert!(after.node_id.is_none());
    let enrolled: EnrollmentResponse = client
        .post(format!(
            "{base}/agent-api/v1/enrollment-requests/{id}/redeem"
        ))
        .json(&RedeemEnrollmentRequest {
            proof: proof("redeem"),
            code: after.code.unwrap(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(enrolled.network_id, own.id);
    let recovered = app.enrollment().poll(id, proof("poll")).await.unwrap();
    assert_eq!(recovered.node_id, Some(enrolled.node_id));
    assert!(recovered.code.is_none());
    assert_eq!(
        client
            .post(format!(
                "{base}/agent-api/v1/enrollment-requests/{id}/cancel"
            ))
            .json(&proof("cancel"))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let execution = app.execution();
    let original_session =
        session(&execution, own.id, enrolled.node_id, &enrolled.keys, &key).await;
    let revision = app
        .networks()
        .get(user.token.expose(), own.id)
        .await
        .unwrap()
        .current_revision;
    let tagged = app
        .configurations()
        .apply(
            user.token.expose(),
            own.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![
                    ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: BTreeMap::from([("tag:server".into(), vec![])]),
                    },
                    ConfigurationOperation::SetIdentityTags {
                        node_id: enrolled.node_id,
                        tags: vec!["tag:server".into()],
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(matches!(
        execution
            .authenticate(original_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    let tag_session = session(&execution, own.id, enrolled.node_id, &enrolled.keys, &key).await;
    let current = app
        .configurations()
        .read(user.token.expose(), own.id, None)
        .await
        .unwrap();
    assert!(!current.bindings.node_users.contains_key(&enrolled.node_id));
    let addresses = current.document.configuration.nodes[&enrolled.node_id]
        .addresses
        .clone();
    let mut forged = current.document.configuration.clone();
    forged.nodes.get_mut(&enrolled.node_id).unwrap().identity = NodeIdentity::User {
        user: "approval@example.com".into(),
    };
    assert!(
        app.configurations()
            .apply(
                user.token.expose(),
                own.id,
                meshway_protocol::configuration::ConfigurationRequest::Document {
                    expected_revision: tagged.revision,
                    request_id: rid(),
                    source_jsonc: WireSecret::new(
                        canonical_json(&forged).unwrap().expose().to_owned()
                    )
                },
                rid()
            )
            .await
            .is_err()
    );
    let relogin = EnrollmentIntent {
        id: EnrollmentRequestId::new(random_uuid().unwrap()).unwrap(),
        control_key: enrolled.keys.control.public_key.clone(),
        dataplane_key: enrolled.keys.dataplane.public_key.clone(),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: "ignored-name".into(),
        existing: Some(ExistingNodeBinding {
            network_id: own.id,
            node_id: enrolled.node_id,
            key_id: enrolled.keys.control.id,
            key_generation: enrolled.keys.control.generation,
        }),
    };
    let relogin_digest = crate::execution::proof::request_digest(&relogin).unwrap();
    let relogin_id = relogin.id;
    app.enrollment()
        .create_request(CreateEnrollmentRequest {
            intent: relogin,
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    key.sign(&enrollment_transcript(relogin_id, relogin_digest, "create"))
                        .to_bytes(),
                ),
            ),
        })
        .await
        .unwrap();
    app.enrollment()
        .approve(
            user.token.expose(),
            relogin_id,
            ApproveEnrollmentRequest { network_id: own.id },
        )
        .await
        .unwrap();
    let relogin_proof = |operation: &str| EnrollmentDeviceProof {
        verifier: WireSecret::new(verifier.clone()),
        signature: WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                key.sign(&enrollment_transcript(
                    relogin_id,
                    relogin_digest,
                    operation,
                ))
                .to_bytes(),
            ),
        ),
    };
    let result = app
        .enrollment()
        .poll(relogin_id, relogin_proof("poll"))
        .await
        .unwrap();
    let recovered = app
        .enrollment()
        .redeem(
            relogin_id,
            RedeemEnrollmentRequest {
                proof: relogin_proof("redeem"),
                code: result.code.unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(recovered.node_id, enrolled.node_id);
    assert_eq!(recovered.keys, enrolled.keys);
    assert!(matches!(
        execution.authenticate(tag_session.token.expose()).await,
        Err(Error::InvalidCredentials)
    ));
    let current = app
        .configurations()
        .read(user.token.expose(), own.id, None)
        .await
        .unwrap();
    assert_eq!(
        current.document.configuration.nodes[&enrolled.node_id].addresses,
        addresses
    );
    assert_eq!(
        current.document.configuration.nodes[&enrolled.node_id].hostname,
        "new-device"
    );
    client
        .post(format!(
            "{base}/api/v1/networks/{}/nodes/{}/keys/revoke",
            own.id, enrolled.node_id
        ))
        .bearer_auth(user.token.expose())
        .json(&RevokeNodeKeyRequest {
            key_id: enrolled.keys.control.id,
            expected_generation: enrolled.keys.control.generation,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let revoked: NodeInventoryView = client
        .get(format!(
            "{base}/api/v1/networks/{}/nodes/{}",
            own.id, enrolled.node_id
        ))
        .bearer_auth(user.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revoked.keys, enrolled.keys);
    assert!(revoked.control_key_revoked_at_ms.is_some());
    assert!(revoked.dataplane_key_revoked_at_ms.is_none());
    assert!(revoked.can_delete);
    let removed: meshway_protocol::configuration::ConfigurationAppliedResponse = client
        .delete(format!(
            "{base}/api/v1/networks/{}/nodes/{}",
            own.id, enrolled.node_id
        ))
        .bearer_auth(user.token.expose())
        .json(&DeleteNodeRequest {
            request_id: rid(),
            expected_revision: current.revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(removed.deleted_node_ids, vec![enrolled.node_id]);
    let attempt = execution
        .challenge(
            execution.signed_challenge_fixture(
                NodeChallengeRequest {
                    network_id: own.id,
                    node_id: enrolled.node_id,
                    key_id: enrolled.keys.control.id,
                    operation: ProofOperation::Session,
                    request_digest: crate::execution::proof::request_digest(&NodeSessionIntent {
                        request_id: rid(),
                    })
                    .unwrap(),
                },
                &key,
            ),
        )
        .await;
    assert!(matches!(attempt, Err(Error::InvalidCredentials)));
    server.abort();
    let _ = server.await;
    app.shutdown().await.unwrap();
}
