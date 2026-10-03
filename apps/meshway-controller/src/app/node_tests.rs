//! SQL-seeded trusted node inventory isolates authentication/execution behavior
//! from the independently-owned Config enrollment commit adapter.
use super::*;
use crate::{
    accounts::{Secret, random_uuid},
    execution::{NodeError, proof},
    secrets::Purpose,
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
    let owner_hash = crate::accounts::token_digest(owner.token.expose()).unwrap();
    let a = NodeId::new(random_uuid().unwrap()).unwrap();
    let b = NodeId::new(random_uuid().unwrap()).unwrap();
    let ak = SigningKey::from_bytes(&[7; 32]);
    let bk = SigningKey::from_bytes(&[8; 32]);
    let ap = ak.verifying_key().to_bytes();
    let bp = bk.verifying_key().to_bytes();
    let sealer = app.sealer();
    let (network, keys) = app
        .executor
        .handle()
        .execute(move |c| {
            let mut prepared =
                crate::networks::configuration::prepare("node tests".into(), &sealer).unwrap();
            let network = prepared.id;
            let mut cfg = crate::networks::configuration::initial(network).unwrap();
            cfg.tags_owners.insert("tag:test".into(), vec![]);
            for (n, name, ip) in [(a, "a", "100.64.0.1"), (b, "b", "100.64.0.2")] {
                cfg.nodes.insert(
                    n,
                    NodeSettings {
                        hostname: name.into(),
                        remark: String::new(),
                        identity: NodeIdentity::Tag {
                            tags: vec!["tag:test".into()],
                        },
                        enabled: true,
                        addresses: NodeAddresses {
                            ipv4: Some(ip.parse().unwrap()),
                            ipv6: None,
                        },
                        listeners: serde_json::from_str(
                            r#"{"udp":{"type":"native_udp","listenPort":23456}}"#,
                        )
                        .unwrap(),
                        dialers: Input::Omitted,
                        exit_enabled: false,
                    },
                );
            }
            let canonical = canonical_json(&cfg).unwrap();
            prepared.source = sealer
                .seal(
                    network,
                    Revision::MIN,
                    Purpose::Source,
                    canonical.expose().as_bytes(),
                )
                .unwrap();
            prepared.configuration = sealer
                .seal(
                    network,
                    Revision::MIN,
                    Purpose::Configuration,
                    canonical.expose().as_bytes(),
                )
                .unwrap();
            store::networks::create(
                c,
                owner_hash,
                prepared,
                rid(),
                &sealer,
                &crate::configuration::SnapshotCache::default(),
            )
            .unwrap();
            let tx = c.transaction().unwrap();
            let now = store::execution::now().unwrap();
            let mut keys = vec![];
            for (node, key, dp) in [(a, ap, [17; 32]), (b, bp, [18; 32])] {
                tx.execute(
                    "INSERT INTO nodes(id,network_id,registered_at) VALUES(?1,?2,?3)",
                    rusqlite::params![
                        node.as_uuid().as_bytes().as_slice(),
                        network.as_uuid().as_bytes().as_slice(),
                        now
                    ],
                )
                .unwrap();
                keys.push(
                    store::execution::insert_keys(
                        &tx,
                        network,
                        node,
                        NodePublicKey {
                            algorithm: KeyAlgorithm::Ed25519,
                            bytes: key.into(),
                        },
                        NodePublicKey {
                            algorithm: KeyAlgorithm::X25519,
                            bytes: dp.into(),
                        },
                        Version::MIN,
                        now,
                    )
                    .unwrap(),
                );
            }
            tx.commit().unwrap();
            (network, keys)
        })
        .await
        .unwrap();
    let mut keys = keys.into_iter();
    (
        dir,
        app,
        network,
        a,
        b,
        keys.next().unwrap(),
        keys.next().unwrap(),
        ak,
        bk,
    )
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
            ConfigurationRequest {
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
            ConfigurationRequest {
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
