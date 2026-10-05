//! Held-key admission to the bounded challenge store, separate from proof use.
use crate::{
    WireSecret,
    node::{NodeChallengeRequest, ProofOperation},
    object::wire_object,
};
use meshway_model::{ChallengeId, KeyId};

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeChallengeIntent {
    pub id: ChallengeId,
    pub controller_id: KeyId,
    pub request: NodeChallengeRequest,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }

wire_object! { pub struct SignedNodeChallengeRequest {
    pub intent: NodeChallengeIntent,
    /// 64-byte Ed25519 signature encoded in unpadded base64url.
    pub signature: WireSecret,
} }

/// Fixed v1 admission transcript. UUIDs are raw 16 bytes, the request digest is
/// raw 32 bytes, and the absolute expiry is a big-endian u64. The operation is
/// one byte (session=1, rotate=2). No JSON encoding or signature bytes enter it.
/// This domain grants challenge allocation only, never consumption of a proof.
pub fn challenge_request_transcript(intent: &NodeChallengeIntent) -> Vec<u8> {
    let mut out = b"meshway/node-challenge-request/ed25519/v1\0".to_vec();
    for id in [
        intent.id.as_uuid(),
        intent.controller_id.as_uuid(),
        intent.request.network_id.as_uuid(),
        intent.request.node_id.as_uuid(),
        intent.request.key_id.as_uuid(),
    ] {
        out.extend_from_slice(id.as_bytes());
    }
    out.push(match intent.request.operation {
        ProofOperation::Session => 1,
        ProofOperation::Rotate => 2,
    });
    out.extend_from_slice(intent.request.request_digest.as_bytes());
    out.extend_from_slice(&intent.expires_at_ms.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_has_fixed_order_width_and_a_separate_admission_domain() {
        let intent = NodeChallengeIntent {
            id: "01010101-0101-0101-0101-010101010101".parse().unwrap(),
            controller_id: "02020202-0202-0202-0202-020202020202".parse().unwrap(),
            request: NodeChallengeRequest {
                network_id: "03030303-0303-0303-0303-030303030303".parse().unwrap(),
                node_id: "04040404-0404-0404-0404-040404040404".parse().unwrap(),
                key_id: "05050505-0505-0505-0505-050505050505".parse().unwrap(),
                operation: ProofOperation::Rotate,
                request_digest: [6; 32].into(),
            },
            expires_at_ms: 0x0102_0304_0506_0708,
        };
        let mut expected = b"meshway/node-challenge-request/ed25519/v1\0".to_vec();
        expected.extend_from_slice(&[1; 16]);
        expected.extend_from_slice(&[2; 16]);
        expected.extend_from_slice(&[3; 16]);
        expected.extend_from_slice(&[4; 16]);
        expected.extend_from_slice(&[5; 16]);
        expected.push(2);
        expected.extend_from_slice(&[6; 32]);
        expected.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(challenge_request_transcript(&intent), expected);
    }
}
