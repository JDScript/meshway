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
pub(super) async fn enroll_tag_node(
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
        .challenge(service.signed_challenge_fixture(
            NodeChallengeRequest {
                network_id: network,
                node_id: node,
                key_id: key.control.id,
                operation: ProofOperation::Session,
                request_digest: proof::request_digest(&intent).unwrap(),
            },
            signer,
        ))
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
            "feature:exit-ipv4".into(),
            Capability::Feature { version: 1 },
        ),
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

mod demands;
mod diagnostics;
mod edge_transports;
mod exit;

mod enrollment_http;

mod enrollment_keys;

mod identity;

mod membership;

mod progress;

mod signatures;

mod state;
