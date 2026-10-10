use super::management::{create, join, net, request, source};
use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::*;
use meshway_model::{MembershipRole, Version};
use meshway_protocol::{configuration::*, history::*, management::*};
use meshway_protocol::enrollment::*;
use sha2::{Digest as _, Sha256};

use super::json as value;

struct Device {
    signer: SigningKey,
    intent: EnrollmentIntent,
    verifier: String,
}
impl Device {
    fn new(seed: u8) -> Self {
        let signer = SigningKey::from_bytes(&[seed; 32]);
        let verifier = "v".repeat(43);
        Self {
            intent: EnrollmentIntent {
                id: EnrollmentRequestId::new(*request().as_uuid()).unwrap(),
                existing: None,
                control_key: NodePublicKey {
                    algorithm: KeyAlgorithm::Ed25519,
                    bytes: signer.verifying_key().to_bytes().into(),
                },
                dataplane_key: NodePublicKey {
                    algorithm: KeyAlgorithm::X25519,
                    bytes: [seed + 40; 32].into(),
                },
                hostname: format!("device-{seed}"),
                code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            },
            signer,
            verifier,
        }
    }
    fn proof(&self, operation: &str) -> EnrollmentDeviceProof {
        let digest = meshway_server::execution::proof::request_digest(&self.intent).unwrap();
        EnrollmentDeviceProof {
            verifier: WireSecret::new(self.verifier.clone()),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    self.signer
                        .sign(&enrollment_transcript(self.intent.id, digest, operation))
                        .to_bytes(),
                ),
            ),
        }
    }
    async fn create(&self, c: &Client, s: &Server) -> EnrollmentRequestView {
        value(
            c.post(format!("{}/agent-api/v1/enrollment-requests", s.url))
                .json(&CreateEnrollmentRequest {
                    intent: self.intent.clone(),
                    signature: self.proof("create").signature,
                }),
        )
        .await
    }
    fn user_url(&self, s: &Server, action: &str) -> String {
        format!(
            "{}/api/v1/device-authorizations/{}{}",
            s.url, self.intent.id, action
        )
    }
    fn device_url(&self, s: &Server, action: &str) -> String {
        format!(
            "{}/agent-api/v1/enrollment-requests/{}/{}",
            s.url, self.intent.id, action
        )
    }
}

mod devices;

mod management;
