//! Real admission/configuration transactions feed authentication and execution.
use super::*;
use crate::{
    accounts::{Secret, random_uuid},
    execution::{NodeError, proof},
    store,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::{execution::*, *};
use meshway_model::*;
use meshway_protocol::{WireSecret, execution::*, node::*};
use std::collections::BTreeMap;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn iid() -> InstanceId {
    InstanceId::new(random_uuid().unwrap()).unwrap()
}
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
async fn fixture() -> (
    tempfile::TempDir,
    App,
    NetworkId,
    NodeId,
    NodeId,
    NodeKeyMetadata,
    NodeKeyMetadata,
    SigningKey,
    SigningKey,
) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let owner = app
        .accounts()
        .register(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let network = app
        .networks()
        .create(owner.token.expose(), "node tests".into(), rid())
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network.id,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: network.current_revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceTagsOwners {
                    tags_owners: BTreeMap::from([("tag:test".into(), vec![])]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let ak = SigningKey::from_bytes(&[7; 32]);
    let bk = SigningKey::from_bytes(&[8; 32]);
    let a = enroll_tag_node(&app, owner.token.expose(), network.id, &ak, "a", [17; 32]).await;
    let b = enroll_tag_node(&app, owner.token.expose(), network.id, &bk, "b", [18; 32]).await;
    (
        dir, app, network.id, a.node_id, b.node_id, a.keys, b.keys, ak, bk,
    )
}
async fn enroll_tag_node(
    app: &App,
    owner: &str,
    network: NetworkId,
    signer: &SigningKey,
    hostname: &str,
    dataplane: [u8; 32],
) -> meshway_protocol::enrollment::EnrollmentResponse {
    use meshway_protocol::enrollment::*;
    use sha2::{Digest as _, Sha256};
    let issued = app
        .enrollment()
        .create_key(
            owner,
            network,
            CreateEnrollmentKeyRequest {
                request_id: rid(),
                tags: vec!["tag:test".into()],
                expires_at_ms: store::execution::now().unwrap() as u64 + 600_000,
                initial_settings: InitialNodeSettings {
                    hostname: Some(hostname.into()),
                    listeners: serde_json::from_str(
                        r#"{"udp":{"type":"native_udp","listenPort":23456}}"#,
                    )
                    .unwrap(),
                },
            },
        )
        .await
        .unwrap();
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
            bytes: dataplane.into(),
        },
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: hostname.into(),
    };
    let signature = signer.sign(&enrollment_transcript(
        intent.id,
        proof::request_digest(&intent).unwrap(),
        "create",
    ));
    app.enrollment()
        .redeem_key(KeyEnrollmentRequest {
            request: CreateEnrollmentRequest {
                intent,
                signature: WireSecret::new(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
            },
            verifier: WireSecret::new(verifier),
            enrollment_key: issued.secret.unwrap(),
        })
        .await
        .unwrap()
}
async fn session(
    service: &crate::execution::Execution,
    network: NetworkId,
    node: NodeId,
    key: &NodeKeyMetadata,
    signer: &SigningKey,
) -> NodeSessionResponse {
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge = service
        .challenge(NodeChallengeRequest {
            network_id: network,
            node_id: node,
            key_id: key.control.id,
            operation: ProofOperation::Session,
            request_digest: proof::request_digest(&intent).unwrap(),
        })
        .await
        .unwrap();
    let signature = URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(&challenge)).to_bytes());
    service
        .session(NodeSessionRequest {
            intent,
            proof: NodeProof {
                challenge_id: challenge.id,
                signature: WireSecret::new(signature),
            },
        })
        .await
        .unwrap()
}
fn caps() -> BTreeMap<String, TransportCapability> {
    BTreeMap::from([(
        "transport:native_udp".into(),
        TransportCapability {
            dial: true,
            listen: true,
            streams: vec!["udp".into()],
        },
    )])
}
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
        Err(NodeError::Unauthorized)
    ));
    let user_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let restored = &current.document.configuration.nodes[&enrolled.node_id];
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
    assert!(matches!(
        app.execution()
            .authenticate(user_session.token.expose())
            .await,
        Err(NodeError::Unauthorized)
    ));
    assert!(matches!(
        app.enrollment().redeem_key(key_request()).await,
        Err(NodeError::Unauthorized)
    ));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
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
    let client = reqwest::Client::new();
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge: NodeChallenge = client
        .post(format!("{base}/agent-api/v1/auth/challenges"))
        .json(&NodeChallengeRequest {
            network_id: network,
            node_id: node,
            key_id: keys.control.id,
            operation: ProofOperation::Session,
            request_digest: proof::request_digest(&intent).unwrap(),
        })
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
    struct Reject;
    impl crate::enrollment::EnrollmentCommit for Reject {
        fn insert_node(
            &self,
            _tx: &rusqlite::Transaction<'_>,
            _admission: &crate::enrollment::NodeAdmission,
        ) -> Result<(), NodeError> {
            Err(NodeError::Invalid)
        }
    }
    assert!(matches!(
        crate::enrollment::Enrollment::new(app.executor.handle(), app.sealer(), Arc::new(Reject))
            .redeem(
                id,
                RedeemEnrollmentRequest {
                    proof: proof("redeem"),
                    code: poll.code.unwrap()
                }
            )
            .await,
        Err(NodeError::Invalid)
    ));
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
        Err(NodeError::Unauthorized)
    ));
    let tag_session = session(&execution, own.id, enrolled.node_id, &enrolled.keys, &key).await;
    let current = app
        .configurations()
        .read(user.token.expose(), own.id, None)
        .await
        .unwrap();
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
        Err(NodeError::Unauthorized)
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
        .challenge(NodeChallengeRequest {
            network_id: own.id,
            node_id: enrolled.node_id,
            key_id: enrolled.keys.control.id,
            operation: ProofOperation::Session,
            request_digest: crate::execution::proof::request_digest(&NodeSessionIntent {
                request_id: rid(),
            })
            .unwrap(),
        })
        .await;
    assert!(matches!(attempt, Err(NodeError::Unauthorized)));
    server.abort();
    let _ = server.await;
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn signed_authentication_replay_rotation_and_controller_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let service = app.execution();
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge = service
        .challenge(NodeChallengeRequest {
            network_id: network,
            node_id: a,
            key_id: keys.control.id,
            operation: ProofOperation::Session,
            request_digest: proof::request_digest(&intent).unwrap(),
        })
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
        Err(NodeError::Unauthorized)
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
        .challenge(NodeChallengeRequest {
            network_id: network,
            node_id: a,
            key_id: keys.control.id,
            operation: ProofOperation::Rotate,
            request_digest: proof::request_digest(&intent).unwrap(),
        })
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
        Err(NodeError::Unauthorized)
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
        Err(NodeError::Unauthorized)
    ));
    let renewed_session = session(&app.execution(), network, a, &rotated, &new).await;
    let resumed = app
        .execution()
        .acquire(
            renewed_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: original_instance.instance_id,
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    assert!(resumed.generation > original_instance.generation);
    assert_ne!(resumed.target, original_instance.target);
    app.check_storage().await.unwrap();
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
        target: target.clone(),
        phase: ReceiptPhase::Applied,
        reported_at_ms: store::execution::now().unwrap() as u64,
        failure: None,
    };
    assert_eq!(
        s.receipt(at.token.expose(), receipt.clone()).await,
        Err(NodeError::Conflict)
    );
    receipt.phase = ReceiptPhase::Saved;
    s.receipt(at.token.expose(), receipt.clone()).await.unwrap();
    receipt.phase = ReceiptPhase::Applied;
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
        Err(NodeError::Conflict)
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
        Err(NodeError::Unauthorized)
    ));
    app.shutdown().await.unwrap();
}
