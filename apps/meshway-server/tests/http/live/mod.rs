//! Public HTTP + real SQLite + signed Node admission. No SQL-created inventory,
//! identity provider, clock substitution or fake execution payload.
use super::management::{create, join, net, request, source};
use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::{state::*, *};
use meshway_protocol::{
    configuration::*, enrollment::*, events::StreamEvent, execution::*, live::*, node::*, state::*,
};
use meshway_server::execution::proof;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub(super) async fn enroll(
    c: &Client,
    s: &Server,
    network: NetworkId,
    token: &str,
    seed: u8,
) -> (EnrollmentResponse, String, InstanceLease) {
    let signer = SigningKey::from_bytes(&[seed; 32]);
    let verifier = "x".repeat(43);
    let intent = EnrollmentIntent {
        id: EnrollmentRequestId::new(*request().as_uuid()).unwrap(),
        existing: None,
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [seed + 20; 32].into(),
        },
        hostname: format!("node-{seed}"),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
    };
    let id = intent.id;
    let digest = proof::request_digest(&intent).unwrap();
    let signature = |operation| {
        WireSecret::new(
            URL_SAFE_NO_PAD.encode(
                signer
                    .sign(&enrollment_transcript(id, digest, operation))
                    .to_bytes(),
            ),
        )
    };
    let _: EnrollmentRequestView = json(
        c.post(format!("{}/agent-api/v1/enrollment-requests", s.url))
            .json(&CreateEnrollmentRequest {
                intent,
                signature: signature("create"),
            }),
    )
    .await;
    let _: DeviceAuthorizationView = json(
        c.post(format!(
            "{}/api/v1/device-authorizations/{id}/approve",
            s.url
        ))
        .bearer_auth(token)
        .json(&ApproveEnrollmentRequest {
            network_id: network,
        }),
    )
    .await;
    let poll: EnrollmentPollResponse = json(
        c.post(format!(
            "{}/agent-api/v1/enrollment-requests/{id}/query",
            s.url
        ))
        .json(&EnrollmentDeviceProof {
            verifier: WireSecret::new(verifier.clone()),
            signature: signature("poll"),
        }),
    )
    .await;
    let enrolled: EnrollmentResponse = json(
        c.post(format!(
            "{}/agent-api/v1/enrollment-requests/{id}/redeem",
            s.url
        ))
        .json(&RedeemEnrollmentRequest {
            proof: EnrollmentDeviceProof {
                verifier: WireSecret::new(verifier),
                signature: signature("redeem"),
            },
            code: poll.code.unwrap(),
        }),
    )
    .await;
    let intent = NodeSessionIntent {
        request_id: request(),
    };
    let allocation = meshway_protocol::NodeChallengeIntent {
        id: ChallengeId::new(*request().as_uuid()).unwrap(),
        controller_id: enrolled.controller_id,
        request: NodeChallengeRequest {
            network_id: network,
            node_id: enrolled.node_id,
            key_id: enrolled.keys.control.id,
            operation: ProofOperation::Session,
            request_digest: proof::request_digest(&intent).unwrap(),
        },
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 30_000,
    };
    let allocation_signature =
        signer.sign(&meshway_protocol::challenge_request_transcript(&allocation));
    let challenge: NodeChallenge = json(
        c.post(format!("{}/agent-api/v1/auth/challenges", s.url))
            .json(&meshway_protocol::SignedNodeChallengeRequest {
                intent: allocation,
                signature: WireSecret::new(URL_SAFE_NO_PAD.encode(allocation_signature.to_bytes())),
            }),
    )
    .await;
    let session: NodeSessionResponse = json(
        c.post(format!("{}/agent-api/v1/auth/session", s.url))
            .json(&NodeSessionRequest {
                intent,
                proof: NodeProof {
                    challenge_id: challenge.id,
                    signature: WireSecret::new(
                        URL_SAFE_NO_PAD
                            .encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
                    ),
                },
            }),
    )
    .await;
    let token = session.token.expose().to_owned();
    let lease: InstanceLease = json(
        c.post(format!("{}/agent-api/v1/instances", s.url))
            .bearer_auth(&token)
            .json(&AcquireInstanceRequest {
                instance_id: InstanceId::new(*request().as_uuid()).unwrap(),
                capabilities: BTreeMap::from([(
                    "transport:native_udp".into(),
                    Capability::Transport {
                        roles: vec![TransportRole::Dial, TransportRole::Listen],
                        streams: vec![StreamKind::Udp],
                    },
                )]),
            }),
    )
    .await;
    (enrolled, token, lease)
}
struct Events {
    response: Response,
    buffer: Vec<u8>,
}
impl Events {
    async fn open(c: &Client, url: String, token: &str, cursor: Option<&str>) -> Self {
        let mut request = c.get(url).bearer_auth(token);
        if let Some(cursor) = cursor {
            request = request.header("last-event-id", cursor);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        assert_headers(&response);
        Self {
            response,
            buffer: vec![],
        }
    }
    async fn frame<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Option<(Option<String>, T, String)> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(end) = self.buffer.windows(2).position(|bytes| bytes == b"\n\n") {
                    let bytes: Vec<_> = self.buffer.drain(..end + 2).collect();
                    let raw = String::from_utf8(bytes).unwrap();
                    if let Some(data) = raw.lines().find_map(|line| line.strip_prefix("data: ")) {
                        let id = raw
                            .lines()
                            .find_map(|line| line.strip_prefix("id: "))
                            .map(str::to_owned);
                        return Some((id, serde_json::from_str(data).unwrap(), raw));
                    }
                } else {
                    match self.response.chunk().await.unwrap() {
                        Some(bytes) => self.buffer.extend_from_slice(&bytes),
                        None => return None,
                    }
                }
            }
        })
        .await
        .expect("bounded SSE delivery")
    }
    async fn user(&mut self) -> (Option<String>, meshway_protocol::UserEvent, String) {
        self.frame().await.expect("user stream remains open")
    }
    async fn ready(&mut self) -> String {
        loop {
            let (id, event, _) = self.user().await;
            if matches!(event.0, StreamEvent::Ready { .. }) {
                return id.unwrap();
            }
        }
    }
}
fn report(lease: &InstanceLease, sequence: u64) -> StateReport {
    StateReport {
        instance_id: lease.instance_id,
        sequence: Sequence::new(sequence).unwrap(),
        observed_at: chrono::Utc::now(),
        state: NodeState {
            system: SystemState {
                hostname: "reported-host".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test-source".into(),
            },
            capabilities: BTreeMap::new(),
            configuration: ConfigurationState::default(),
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState::default(),
            probes: vec![],
            problems: vec![],
        },
    }
}

mod execution;

mod reports;

mod visibility;
