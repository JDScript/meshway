//! Real admission/configuration transactions feed authentication and execution.
use super::*;
use crate::error::Error;
use crate::{
    accounts::{Secret, random_uuid},
    execution::proof,
    store,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::{execution::*, *};
use meshway_model::state::{Capability, StreamKind, TransportRole};
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
pub(super) async fn fixture() -> (
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
pub(super) async fn session(
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
pub(super) fn caps() -> BTreeMap<String, Capability> {
    BTreeMap::from([
        (
            "feature:path-observation".into(),
            Capability::Feature { version: 1 },
        ),
        (
            "transport:native_udp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Udp],
            },
        ),
    ])
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
pub(super) async fn enroll_user_node(
    app: &App,
    token: &str,
    network: NetworkId,
    signer: &SigningKey,
    dp: [u8; 32],
) -> meshway_protocol::enrollment::EnrollmentResponse {
    try_enroll_user_node(app, token, network, signer, dp)
        .await
        .unwrap()
}
async fn try_enroll_user_node(
    app: &App,
    token: &str,
    network: NetworkId,
    signer: &SigningKey,
    dp: [u8; 32],
) -> Result<meshway_protocol::enrollment::EnrollmentResponse, Error> {
    use meshway_protocol::enrollment::*;
    use sha2::{Digest as _, Sha256};
    let verifier = "p".repeat(43);
    let intent = EnrollmentIntent {
        id: EnrollmentRequestId::new(random_uuid().unwrap()).unwrap(),
        existing: None,
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: dp.into(),
        },
        hostname: "member-node".into(),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
    };
    let id = intent.id;
    let digest = proof::request_digest(&intent).unwrap();
    let signature = |op: &str| {
        WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                signer
                    .sign(&enrollment_transcript(id, digest, op))
                    .to_bytes(),
            ),
        )
    };
    app.enrollment()
        .create_request(CreateEnrollmentRequest {
            intent,
            signature: signature("create"),
        })
        .await
        .unwrap();
    app.enrollment()
        .approve(
            token,
            id,
            ApproveEnrollmentRequest {
                network_id: network,
            },
        )
        .await
        .unwrap();
    let device_proof = |op: &str| EnrollmentDeviceProof {
        verifier: WireSecret::new(verifier.clone()),
        signature: signature(op),
    };
    let code = app
        .enrollment()
        .poll(id, device_proof("poll"))
        .await
        .unwrap()
        .code
        .unwrap();
    app.enrollment()
        .redeem(
            id,
            RedeemEnrollmentRequest {
                proof: device_proof("redeem"),
                code,
            },
        )
        .await
}

#[tokio::test]
async fn member_removal_atomically_deletes_user_nodes_and_preserves_tag_nodes() {
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
    let node_session = session(
        &app.execution(),
        network,
        enrolled.node_id,
        &enrolled.keys,
        &signer,
    )
    .await;
    let impact = app
        .networks()
        .removal_impact(member.token.expose(), network, None, rid())
        .await
        .unwrap();
    assert_eq!(impact.node_ids, vec![enrolled.node_id]);
    let event_sequence = app.state_runtime.lock().unwrap().hub.events.sequence();
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
    assert_eq!(
        app.networks()
            .get(owner.token.expose(), network)
            .await
            .unwrap()
            .current_revision,
        impact.revision
    );
    assert!(
        app.state_runtime
            .lock()
            .unwrap()
            .hub
            .events
            .after(event_sequence, network)
            .unwrap()
            .is_empty()
    );
    app.execution()
        .authenticate(node_session.token.expose())
        .await
        .unwrap();
    app.executor
        .handle()
        .execute(|db| db.execute_batch("DROP TRIGGER fail_member_removal;"))
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
    let changes = app
        .state_runtime
        .lock()
        .unwrap()
        .hub
        .events
        .after(event_sequence, network)
        .unwrap();
    assert_eq!(changes.iter().filter(|entry| matches!(entry.change, crate::effect::Effect::NodeRetired { node_id, .. } if node_id == enrolled.node_id)).count(),1);
    assert!(!changes.iter().any(|entry| matches!(entry.change, crate::effect::Effect::NodeRetired { node_id, .. } if node_id == tag)));
    assert!(matches!(
        app.execution()
            .authenticate(node_session.token.expose())
            .await,
        Err(Error::InvalidCredentials)
    ));
    let inventory = app
        .execution()
        .inventory(owner.token.expose(), network)
        .await
        .unwrap();
    assert!(!inventory.items.iter().any(|n| n.id == enrolled.node_id));
    assert!(inventory.items.iter().any(|n| n.id == tag));
    assert!(
        app.networks()
            .get(member.token.expose(), network)
            .await
            .is_err()
    );
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(
        !snapshot
            .document
            .configuration
            .nodes
            .contains_key(&enrolled.node_id)
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

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
async fn a_node_may_report_the_configuration_it_saved_just_before_the_next_revisions() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
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
    let saved = ai.target.clone().unwrap();
    // Two more revisions land before the node reports what it saved.
    for remark in ["first", "second"] {
        let revision = app
            .configurations()
            .read(owner.token.expose(), network, None)
            .await
            .unwrap()
            .revision;
        app.configurations()
            .apply(
                owner.token.expose(),
                network,
                meshway_protocol::configuration::ConfigurationRequest::Operations {
                    expected_revision: revision,
                    request_id: rid(),
                    ops: vec![ConfigurationOperation::SetNodeSettings {
                        node_id: a,
                        fields: meshway_configuration::NodeSettingsPatch {
                            remark: meshway_configuration::Input::Value(remark.into()),
                            ..Default::default()
                        },
                    }],
                },
                rid(),
            )
            .await
            .unwrap();
    }
    let current = service
        .renew(
            a_session.token.expose(),
            InstanceRequest {
                instance_id: ai.instance_id,
                generation: ai.generation,
            },
        )
        .await
        .unwrap()
        .target
        .unwrap();
    assert_ne!(current, saved);
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let report = s
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            // The saved reference is admitted as history: readable, never an
            // installation or execution authority.
            assert!(report.configurations.contains(&saved));
            assert!(!report.context(&saved).unwrap().installation_allowed);
            assert_eq!(report.execution.as_ref(), Some(&current));
            assert!(report.context(&current).unwrap().installation_allowed);
            // A reference the node never held is still not admitted.
            let invented = ConfigurationRef {
                digest: [9; 32].into(),
                ..saved.clone()
            };
            assert!(matches!(
                report.context(&invented),
                Err(Error::InvalidRequest)
            ));
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn state_admission_uses_real_node_instance_and_current_publication_dependencies() {
    use crate::error::Error;
    use crate::state::service::ReadIdentity;
    use meshway_model::state::ResourceRef;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let a_session = session(&service, network, a, &ak, &signer_a).await;
    let b_session = session(&service, network, b, &bk, &signer_b).await;
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
    service
        .acquire(
            b_session.token.expose(),
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
    service
        .demand(
            a_session.token.expose(),
            DemandRequest {
                instance: instance.clone(),
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
    let lease = service
        .renew(a_session.token.expose(), instance.clone())
        .await
        .unwrap();
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    let expected = lease.target.clone();
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let owner_digest = crate::accounts::token_digest(owner.token.expose()).unwrap();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let now = chrono::Utc::now();
            let report = s.report_admission_in_transaction(&tx, &token, now).unwrap();
            assert_eq!(report.execution, expected);
            assert_eq!(report.instance_id, ai.instance_id);
            assert!(report.current().unwrap().peers.contains(&b));
            assert!(
                report
                    .current()
                    .unwrap()
                    .paths
                    .iter()
                    .all(|p| p.first() == Some(&a) && p.last() == Some(&b))
            );
            assert!(
                report
                    .current()
                    .unwrap()
                    .resources
                    .contains_key(&ResourceRef::Listener { key: "udp".into() })
            );
            assert!(
                report
                    .current()
                    .unwrap()
                    .resources
                    .iter()
                    .any(|(r, v)| matches!(r, ResourceRef::Dial { .. })
                        && v.configured_endpoints.is_empty())
            );
            let scope = s
                .state_scope_in_transaction(
                    &tx,
                    &ReadIdentity::Node {
                        authorization: token,
                    },
                    now,
                )
                .unwrap();
            assert_eq!(scope.visible, [a, b].into_iter().collect());
            assert_eq!(scope.full, [a].into_iter().collect());
            assert!(
                scope.current[&a]
                    .configurations
                    .contains(report.execution.as_ref().unwrap())
            );
            let identity = ReadIdentity::UserSession {
                digest: owner_digest,
                network,
            };
            let request = meshway_protocol::observations::CreateObservation {
                service: meshway_configuration::execution::ObservationServiceScope::Tcp {
                    port: meshway_transport_spec::Port::new(443).unwrap(),
                },
                subscription_id: ObservationSubscriptionId::new(random_uuid().unwrap()).unwrap(),
                source_node_id: a,
                target_node_id: b,
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a, b],
                },
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            };
            let admitted = crate::observations::service::ObservationProvider::prepare_observation(
                &s, &tx, &identity, &request, now,
            )
            .unwrap();
            assert_eq!(admitted.paths, vec![vec![a, b]]);
            let invalid = meshway_protocol::observations::CreateObservation {
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a, b, a],
                },
                ..request
            };
            assert!(matches!(
                s.observation_admission_in_transaction(&tx, &identity, &invalid, now),
                Err(Error::PermissionDenied)
            ));
        })
        .await
        .unwrap();
    service
        .acquire(
            a_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    let previous = lease.target.unwrap();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let admission = s
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            assert_ne!(admission.instance_id, instance.instance_id);
            // The target the previous instance held is recent history the new
            // instance may still report as saved (it survives on disk), never
            // an installation or execution context.
            assert!(admission.configurations.contains(&previous));
            assert!(!admission.context(&previous).unwrap().installation_allowed);
            assert_ne!(admission.execution.as_ref(), Some(&previous));
            assert!(admission.current().unwrap().peers.is_empty());
            assert!(matches!(
                s.report_admission_in_transaction(
                    &tx,
                    "mws_not-a-node-session",
                    chrono::Utc::now()
                ),
                Err(Error::InvalidCredentials)
            ));
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
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
    assert!(matches!(attempt, Err(Error::InvalidCredentials)));
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
        Err(Error::BindingConflict)
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
async fn review_demand_retry_preserves_peer_target_and_lease() {
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
    let request = DemandRequest {
        instance: ar,
        request_id: rid(),
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 60,
    };
    let first = s.demand(at.token.expose(), request.clone()).await.unwrap();
    let peer = s.renew(bt.token.expose(), br.clone()).await.unwrap();
    let retry = s.demand(at.token.expose(), request).await.unwrap();
    assert_eq!(retry, first);
    let after = s
        .configuration(
            bt.token.expose(),
            NodeConfigurationRequest {
                instance: br,
                base: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(after.lease, peer);
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
        .challenge(NodeChallengeRequest {
            network_id: network,
            node_id: a,
            key_id: keys.control.id,
            operation: ProofOperation::Rotate,
            request_digest: proof::request_digest(&intent).unwrap(),
        })
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
    let client = reqwest::Client::new();
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
        Err(Error::BindingConflict)
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
        Err(Error::BindingConflict)
    );
    let mut invalid = receipt.clone();
    invalid.instance.instance_id = iid();
    assert_eq!(
        s.receipt(at.token.expose(), invalid).await,
        Err(Error::BindingConflict)
    );
    let mut invalid = receipt.clone();
    invalid.instance.generation = Version::new(receipt.instance.generation.get() + 1).unwrap();
    assert_eq!(
        s.receipt(at.token.expose(), invalid).await,
        Err(Error::BindingConflict)
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
        Err(Error::BindingConflict)
    );
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
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
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

#[tokio::test]
async fn real_multihop_admission_separates_neighbors_paths_and_receipt_history() {
    use meshway_model::state::{ProbeKind, ProbeObservation, ProbeOutcome};
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
    let csigner = SigningKey::from_bytes(&[63; 32]);
    let c = enroll_tag_node(
        &app,
        owner.token.expose(),
        network,
        &csigner,
        "relay",
        [73; 32],
    )
    .await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.defaults.topology = TopologyDefault::Deny;
    config.topology = [(a, c.node_id), (c.node_id, b)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
        })
        .collect();
    config.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        via: vec![format!("node:{}", c.node_id)],
    }];
    config.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
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
    let request = DemandRequest {
        instance: instances[0].clone(),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    s.demand(sessions[0].token.expose(), request.clone())
        .await
        .unwrap();
    let mut leases = Vec::new();
    for (session, instance) in sessions.iter().zip(&instances) {
        leases.push(
            s.renew(session.token.expose(), instance.clone())
                .await
                .unwrap(),
        );
    }
    let old = leases[0].target.clone().unwrap();
    s.receipt(
        sessions[0].token.expose(),
        ReceiptRequest {
            instance: instances[0].clone(),
            target: old.clone(),
            phase: ReceiptPhase::Saved,
            attempt: Version::MIN,
            sequence: Sequence::MIN,
            reported_at_ms: store::execution::now().unwrap() as u64,
            failure: None,
        },
    )
    .await
    .unwrap();
    let tokens: Vec<_> = sessions
        .iter()
        .map(|session| session.token.expose().to_owned())
        .collect();
    let executor = s.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let now = chrono::Utc::now();
            for (index, node, peer) in [(0, a, c.node_id), (2, c.node_id, b)] {
                let admission = executor
                    .report_admission_in_transaction(&tx, &tokens[index], now)
                    .unwrap();
                let mut probe = ProbeObservation {
                    measurement_id: EventId::new(random_uuid().unwrap()).unwrap(),
                    kind: ProbeKind::Neighbor,
                    link_id: Some("reported-channel".into()),
                    transport: Some("native_udp".into()),
                    path: vec![node, peer],
                    publication_id: admission.execution.as_ref().unwrap().execution_publication,
                    started_at: now,
                    completed_at: Some(now),
                    outcome: ProbeOutcome::Success,
                    duration_us: Some(Sequence::new(12).unwrap()),
                };
                crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                // Reproduce the former B projection: business paths alone cannot
                // authorize these adjacent transport probes.
                let mut missing = admission.clone();
                for context in &mut missing.contexts {
                    context.neighbor_transports.clear();
                }
                assert!(crate::state::validation::validate_probe(&missing, &probe, now).is_err());
                probe.kind = ProbeKind::Connect;
                crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                probe.transport = Some("vless".into());
                assert!(crate::state::validation::validate_probe(&admission, &probe, now).is_err());
                probe.kind = ProbeKind::Path;
                probe.transport = None;
                probe.link_id = None;
                assert!(crate::state::validation::validate_probe(&admission, &probe, now).is_err());
                if node == a {
                    probe.path = vec![a, c.node_id, b];
                    crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                }
            }
        })
        .await
        .unwrap();
    let mut second = request;
    second.request_id = rid();
    s.demand(sessions[0].token.expose(), second).await.unwrap();
    let token = sessions[0].token.expose().to_owned();
    let executor = s.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let admission = executor
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            assert_ne!(admission.execution.as_ref(), Some(&old));
            assert!(admission.current().unwrap().installation_allowed);
            assert!(!admission.context(&old).unwrap().installation_allowed);
            assert_eq!(
                admission.context(&old).unwrap().paths,
                vec![vec![a, c.node_id, b]]
            );
        })
        .await
        .unwrap();
    let rejected = s
        .demand(
            sessions[2].token.expose(),
            DemandRequest {
                instance: instances[2].clone(),
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
    assert!(matches!(
        rejected.decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejoined_user_enrollment_preserves_inactive_group_membership() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, target, _, target_keys, _, target_signer, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let user = app
        .accounts()
        .register(
            "rejoining@example.test".into(),
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
    let (_, old_membership) = app
        .networks()
        .redeem_join_link(user.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let old_signer = SigningKey::from_bytes(&[81; 32]);
    let old_node =
        enroll_user_node(&app, user.token.expose(), network, &old_signer, [82; 32]).await;
    let old_session = session(
        &app.execution(),
        network,
        old_node.node_id,
        &old_node.keys,
        &old_signer,
    )
    .await;
    let snapshot = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: snapshot.revision,
                request_id: rid(),
                ops: vec![
                    ConfigurationOperation::ReplaceGroups {
                        groups: BTreeMap::from([(
                            "group:former-members".into(),
                            vec![user.session.user.email.clone()],
                        )]),
                    },
                    ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: BTreeMap::from([
                            ("tag:test".into(), vec![]),
                            (
                                "tag:former-owned".into(),
                                vec![user.session.user.email.clone()],
                            ),
                        ]),
                    },
                    ConfigurationOperation::ReplaceAccess {
                        access: vec![AccessRule {
                            remark: String::new(),
                            src: vec!["group:former-members".into()],
                            dst: vec![format!("node:{target}")],
                            ip: vec!["tcp:443".into()],
                            bidirectional: false,
                        }],
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    let impact = app
        .networks()
        .removal_impact(owner.token.expose(), network, Some(old_membership), rid())
        .await
        .unwrap();
    assert_eq!(impact.node_ids, vec![old_node.node_id]);
    app.networks()
        .remove_member(
            owner.token.expose(),
            network,
            Some(old_membership),
            impact.version,
            impact.revision,
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
    let (_, new_membership) = app
        .networks()
        .redeem_join_link(user.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    assert_ne!(old_membership, new_membership);
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        before.bindings.users[&user.session.user.email].membership_id,
        old_membership
    );
    assert!(
        !before
            .document
            .configuration
            .nodes
            .contains_key(&old_node.node_id)
    );
    let signer = SigningKey::from_bytes(&[83; 32]);
    let result = try_enroll_user_node(&app, user.token.expose(), network, &signer, [84; 32]).await;
    if result.is_err() {
        let unchanged = app
            .configurations()
            .read(owner.token.expose(), network, None)
            .await
            .unwrap();
        assert_eq!(unchanged.revision, before.revision);
        assert_eq!(unchanged.bindings.users, before.bindings.users);
        assert_eq!(
            unchanged.document.configuration,
            before.document.configuration
        );
        app.check_storage().await.unwrap();
    }
    // System enrollment must neither reject the current member due to an old
    // group reference nor reactivate that group's old membership binding.
    let enrolled =
        result.expect("current membership must enroll without rebinding the inactive group");
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        after.document.configuration.groups,
        before.document.configuration.groups
    );
    assert_eq!(after.bindings.users, before.bindings.users);
    assert_eq!(
        after.bindings.node_users[&enrolled.node_id],
        UserBinding {
            user_id: user.session.user.id,
            membership_id: new_membership
        }
    );
    assert!(!after.bindings.node_users.contains_key(&old_node.node_id));
    assert!(!after.bindings.node_users.contains_key(&target));
    let comparison = app
        .configurations()
        .compare(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::CompareConfigurationRequest {
                from_revision: before.revision,
                to_revision: after.revision,
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(
        comparison
            .changes
            .iter()
            .any(|change| change.path == "/bindings/nodeUsers")
    );
    assert!(
        !comparison
            .changes
            .iter()
            .any(|change| change.path == "/bindings/users")
    );
    let history = app
        .configurations()
        .read(owner.token.expose(), network, Some(snapshot.revision))
        .await
        .unwrap();
    assert_eq!(
        history.bindings.node_users[&old_node.node_id].membership_id,
        old_membership
    );
    assert!(
        app.configurations()
            .apply(
                owner.token.expose(),
                network,
                meshway_protocol::configuration::ConfigurationRequest::Restore {
                    expected_revision: after.revision,
                    resource_publication: None,
                    request_id: rid(),
                    restore_from: snapshot.revision
                },
                rid()
            )
            .await
            .is_err()
    );
    let service = app.execution();
    assert_eq!(
        service
            .challenge(NodeChallengeRequest {
                network_id: network,
                node_id: old_node.node_id,
                key_id: old_node.keys.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&NodeSessionIntent { request_id: rid() })
                    .unwrap()
            })
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    let node_session = session(&service, network, enrolled.node_id, &enrolled.keys, &signer).await;
    let target_name = node_dns_name(
        &after.document.configuration.nodes[&target].hostname,
        &after.document.configuration.dns.domain,
    )
    .unwrap();
    assert!(matches!(
        service
            .query_dns(
                old_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    // Rejoining does not reactivate the former membership's group grant.
    assert!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await
            .unwrap()
            .record
            .is_none()
    );
    let lease = service
        .acquire(
            node_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let decision = service
        .demand(
            node_session.token.expose(),
            DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: rid(),
                target: BusinessTarget::Mesh { node: target },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        decision.decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    // An ordinary source save keeps both purposes; it cannot rebind the policy
    // slot to M2 merely because a new Node now has the same readable email.
    let saved = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: after.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(
                    canonical_json(&after.document.configuration)
                        .unwrap()
                        .expose()
                        .into(),
                ),
            },
            rid(),
        )
        .await
        .unwrap();
    let preview = app
        .configurations()
        .validate(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: saved.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::SetNodeSettings {
                    node_id: enrolled.node_id,
                    fields: NodeSettingsPatch {
                        remark: Input::Value("new membership device".into()),
                        ..Default::default()
                    },
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    assert!(
        preview
            .unavailable_dependencies
            .iter()
            .any(|issue| issue.path.starts_with("/tagsOwners/"))
    );
    let target_session = session(&service, network, target, &target_keys, &target_signer).await;
    service
        .acquire(
            target_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let authorized = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: saved.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceAccess {
                    access: vec![AccessRule {
                        remark: String::new(),
                        src: vec![format!("node:{}", enrolled.node_id)],
                        dst: vec![format!("node:{target}")],
                        ip: vec!["tcp:443".into()],
                        bidirectional: false,
                    }],
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let permitted = service
        .demand(
            node_session.token.expose(),
            DemandRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                request_id: rid(),
                target: BusinessTarget::Mesh { node: target },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        permitted.decision,
        DemandDecision::Permitted { .. }
    ));
    let preserved = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(preserved.bindings.users, before.bindings.users);
    assert_eq!(
        preserved.bindings.node_users[&enrolled.node_id].membership_id,
        new_membership
    );
    let impact = app
        .networks()
        .removal_impact(owner.token.expose(), network, Some(new_membership), rid())
        .await
        .unwrap();
    assert_eq!(impact.revision, authorized.revision);
    assert_eq!(impact.node_ids, vec![enrolled.node_id]);
    assert_eq!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest {
                    name: target_name.clone()
                },
                rid()
            )
            .await
            .unwrap()
            .record
            .unwrap()
            .node_id,
        target
    );
    app.networks()
        .remove_member(
            owner.token.expose(),
            network,
            Some(new_membership),
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .authenticate(node_session.token.expose())
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    let removed = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(!removed.bindings.node_users.contains_key(&enrolled.node_id));
    assert_eq!(removed.bindings.users, before.bindings.users);
    assert!(matches!(
        service
            .query_dns(
                node_session.token.expose(),
                NodeDnsQueryRequest { name: target_name },
                rid()
            )
            .await,
        Err(Error::InvalidCredentials)
    ));
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn withdrawing_execution_dependencies_commits_and_never_falls_through_to_direct() {
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
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.nodes.get_mut(&b).unwrap().exit_enabled = true;
    config.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["1.1.1.1/32".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    let rule_sets = BTreeMap::from([(
        "destinations".into(),
        RuleSet::Inline {
            ip_cidr: vec!["1.1.1.1/32".into()],
            domain_suffix: vec![],
        },
    )]);
    config.rule_sets = rule_sets.clone();
    config.routing.egress.rules = vec![
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                rule_sets: vec!["destinations".into()],
                ..Default::default()
            },
            action: RouteAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        },
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["1.1.1.1/32".into()],
                ..Default::default()
            },
            action: RouteAction::Direct,
        },
    ];
    let applied = app
        .configurations()
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
    let mut revision = applied.revision;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &asigner).await;
    let bt = session(&service, network, b, &bk, &bsigner).await;
    let ai = service
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    service
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let request = DemandRequest {
        instance: InstanceRequest {
            instance_id: ai.instance_id,
            generation: ai.generation,
        },
        request_id: rid(),
        target: BusinessTarget::Ip {
            address: ExecutionAddress("1.1.1.1".parse().unwrap()),
        },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    assert!(matches!(
        service
            .demand(at.token.expose(), request.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Permitted { .. }
    ));
    for (ops, reason) in [
        (
            vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
            Some(UnavailableReason::Unauthorized),
        ),
        (
            vec![ConfigurationOperation::ReplaceAccess {
                access: config.access.clone(),
            }],
            None,
        ),
        (
            vec![ConfigurationOperation::SetNodeSettings {
                node_id: b,
                fields: NodeSettingsPatch {
                    exit_enabled: Input::Value(false),
                    ..Default::default()
                },
            }],
            Some(UnavailableReason::NoPath),
        ),
        (
            vec![ConfigurationOperation::DeleteNode { node_id: b }],
            Some(UnavailableReason::NoPath),
        ),
    ] {
        let update = || meshway_protocol::configuration::ConfigurationRequest::Operations {
            expected_revision: revision,
            request_id: rid(),
            ops: ops.clone(),
        };
        let preview = app
            .configurations()
            .validate(owner.token.expose(), network, update(), rid())
            .await
            .unwrap();
        if reason.is_some() {
            assert!(!preview.unavailable_dependencies.is_empty());
        }
        revision = app
            .configurations()
            .apply(owner.token.expose(), network, update(), rid())
            .await
            .unwrap()
            .revision;
        let decision = service
            .demand(at.token.expose(), request.clone())
            .await
            .unwrap()
            .decision;
        match reason {
            Some(reason) => assert_eq!(decision, DemandDecision::Unavailable { reason }),
            None => assert!(matches!(decision, DemandDecision::Permitted { .. })),
        }
    }
    assert_eq!(
        service.authenticate(bt.token.expose()).await.err(),
        Some(Error::InvalidCredentials)
    );
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(after.document.configuration.routing, config.routing);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn email_maintenance_preserves_live_node_subject_and_session() {
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
    let signer = SigningKey::from_bytes(&[85; 32]);
    let peer = app
        .accounts()
        .register(
            "pupil@example.com".into(),
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
    app.networks()
        .redeem_join_link(peer.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let node = enroll_user_node(&app, owner.token.expose(), network, &signer, [86; 32]).await;
    let service = app.execution();
    let session = session(&service, network, node.node_id, &node.keys, &signer).await;
    let lease = service
        .acquire(
            session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: before.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceGroups {
                    groups: BTreeMap::from([(
                        "group:team".into(),
                        vec!["owner@example.com".into(), "pupil@example.com".into()],
                    )]),
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    let changed = app
        .accounts()
        .change_email(
            owner.token.expose(),
            Secret::new("a sufficiently long password".into()),
            "renamed-owner@example.com".into(),
            rid(),
            app.configurations().committer(),
        )
        .await
        .unwrap();
    assert_eq!(changed.id, owner.session.user.id);
    assert_eq!(
        service
            .authenticate(session.token.expose())
            .await
            .unwrap()
            .node_id(),
        node.node_id
    );
    let next = service
        .renew(
            session.token.expose(),
            InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
        )
        .await
        .unwrap();
    assert_eq!(next.instance_id, lease.instance_id);
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(
        before.bindings.node_users[&node.node_id],
        after.bindings.node_users[&node.node_id]
    );
    assert_eq!(
        after.document.configuration.nodes[&node.node_id].identity,
        NodeIdentity::User {
            user: "renamed-owner@example.com".into()
        }
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn node_only_email_reference_and_profile_selection_survive_history_restore() {
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
    let signer = SigningKey::from_bytes(&[87; 32]);
    let enrolled = enroll_user_node(&app, owner.token.expose(), network, &signer, [88; 32]).await;
    let base = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let routing=serde_json::from_value(serde_json::json!({"mesh":[],"egress":{"rules":[],"profiles":{"work":{"rules":[],"fallback":{"type":"unmanaged"}}},"assignments":[{"target":[format!("node:{}",enrolled.node_id)],"allowed":["mesh","work"],"default":"mesh","locked":false}]}})).unwrap();
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: base.revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::ReplaceRouting { routing }],
            },
            rid(),
        )
        .await
        .unwrap();
    let before = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(before.bindings.users.is_empty());
    let service = app.execution();
    let session = session(&service, network, enrolled.node_id, &enrolled.keys, &signer).await;
    let lease = service
        .acquire(
            session.token.expose(),
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
    service
        .select_profile(
            session.token.expose(),
            RoutingSelectionRequest {
                instance: instance.clone(),
                expected_version: None,
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    app.accounts()
        .change_email(
            owner.token.expose(),
            Secret::new("a sufficiently long password".into()),
            "node-only@example.test".into(),
            rid(),
            app.configurations().committer(),
        )
        .await
        .unwrap();
    let changed = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert!(changed.bindings.users.is_empty());
    assert_eq!(changed.bindings.node_users, before.bindings.node_users);
    assert_eq!(
        changed.document.configuration.nodes[&enrolled.node_id].identity,
        NodeIdentity::User {
            user: "node-only@example.test".into()
        }
    );
    let full = service
        .configuration(
            session.token.expose(),
            NodeConfigurationRequest {
                instance: instance.clone(),
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap();
    assert_eq!(full.configuration.dns.profile, "work");
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Restore {
                expected_revision: changed.revision,
                resource_publication: None,
                request_id: rid(),
                restore_from: before.revision,
            },
            rid(),
        )
        .await
        .unwrap();
    let restored = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(restored.bindings.node_users, before.bindings.node_users);
    assert_eq!(
        restored.document.configuration.nodes[&enrolled.node_id].identity,
        changed.document.configuration.nodes[&enrolled.node_id].identity
    );
    let history = app
        .configurations()
        .read(owner.token.expose(), network, Some(before.revision))
        .await
        .unwrap();
    assert_eq!(
        history.document.configuration.nodes[&enrolled.node_id].identity,
        NodeIdentity::User {
            user: "owner@example.com".into()
        }
    );
    let full = service
        .configuration(
            session.token.expose(),
            NodeConfigurationRequest {
                instance,
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap();
    assert_eq!(full.configuration.dns.profile, "work");
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
