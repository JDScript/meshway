//! Ed25519 challenge authentication. Sign `proof_transcript`, not JSON serialization.
use crate::{WireSecret, object::wire_object};
use meshway_model::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProofOperation {
    Session,
    Rotate,
}
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeChallengeRequest {
    pub network_id: NetworkId, pub node_id: NodeId, pub key_id: KeyId,
    pub operation: ProofOperation, pub request_digest: Digest,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeChallenge {
    pub id: ChallengeId, pub controller_id: KeyId, pub network_id: NetworkId,
    pub node_id: NodeId, pub key_id: KeyId, pub key_generation: Version,
    pub nonce: Digest, pub operation: ProofOperation, pub request_digest: Digest,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }
wire_object! { pub struct NodeProof {
    pub challenge_id: ChallengeId,
    /// 64-byte Ed25519 signature encoded in unpadded base64url.
    pub signature: WireSecret,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeSessionIntent {
    pub request_id: RequestId,
} }
wire_object! { pub struct NodeSessionRequest { pub intent: NodeSessionIntent, pub proof: NodeProof } }
wire_object! { pub struct NodeSessionResponse {
    pub node_id: NodeId, pub network_id: NetworkId, pub key_id: KeyId,
    pub token: WireSecret,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeRotationIntent {
    pub request_id: RequestId, pub control_key: NodePublicKey, pub dataplane_key: NodePublicKey,
} }
wire_object! { pub struct NodeRotationRequest {
    pub intent: NodeRotationIntent, pub old_proof: NodeProof,
    pub new_signature: WireSecret,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeKeyMetadata {
    pub control: BoundNodeKey, pub dataplane: BoundNodeKey,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeInventoryView {
    pub id: NodeId, pub network_id: NetworkId,
    #[serde(rename="registeredAt", with="meshway_model::timestamp")]
    pub registered_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
    pub hostname: String, pub enabled: bool, pub identity: meshway_configuration::NodeIdentity,
    pub addresses: meshway_configuration::NodeAddresses, pub keys: NodeKeyMetadata,
    /// Revocation of the displayed latest keys; revoked configured nodes remain manageable.
    #[serde(rename="controlKeyRevokedAt", with="meshway_model::timestamp::optional")]
    pub control_key_revoked_at_ms: Option<u64> [value_type=Option<chrono::DateTime<chrono::Utc>>,required=true],
    #[serde(rename="dataplaneKeyRevokedAt", with="meshway_model::timestamp::optional")]
    pub dataplane_key_revoked_at_ms: Option<u64> [value_type=Option<chrono::DateTime<chrono::Utc>>,required=true],
    pub can_delete: bool,
    /// Current execution target, not a saved/applied receipt. May be a later refresh than the source apply.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub desired_execution: Option<ConfigurationRef> [required=true],
    pub can_revoke_control_key: bool,
    pub can_revoke_dataplane_key: bool,
    #[serde(rename="controlKeyCreatedAt", with="meshway_model::timestamp")]
    pub control_key_created_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
    #[serde(rename="dataplaneKeyCreatedAt", with="meshway_model::timestamp")]
    pub dataplane_key_created_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeInventoryResponse {
    pub items: Vec<NodeInventoryView>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeSettingsResponse {
    pub request_id: RequestId, pub network_id: NetworkId, pub node_id: NodeId,
    pub revision: Revision, pub settings: meshway_configuration::NodeSettingsView,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct DeleteNodeRequest {
    pub request_id: RequestId, pub expected_revision: Revision,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RevokeNodeKeyRequest {
    pub key_id: KeyId, pub expected_generation: Version,
} }

/// Fixed v1 transcript: domain || operation byte || fixed-width binding fields.
/// u64 integers are big-endian; UUIDs are raw 16 bytes; digests are raw 32 bytes.
/// The controller ID is the durable installation key ID, pinned alongside the
/// HTTPS origin by the client. TLS authentication is still mandatory remotely.
pub fn proof_transcript(c: &NodeChallenge) -> Vec<u8> {
    let mut out = b"meshway/node-proof/ed25519/v1\0".to_vec();
    out.push(match c.operation {
        ProofOperation::Session => 1,
        ProofOperation::Rotate => 2,
    });
    for id in [
        c.controller_id.as_uuid(),
        c.network_id.as_uuid(),
        c.node_id.as_uuid(),
        c.key_id.as_uuid(),
        c.id.as_uuid(),
    ] {
        out.extend_from_slice(id.as_bytes());
    }
    out.extend_from_slice(&c.key_generation.get().to_be_bytes());
    out.extend_from_slice(c.nonce.as_bytes());
    out.extend_from_slice(&c.expires_at_ms.to_be_bytes());
    out.extend_from_slice(c.request_digest.as_bytes());
    out
}

/// Digest of an already typed intent, independent of struct/property order.
/// This encoding is shared by the signing Node and verifying Server.
pub fn request_digest<T: Serialize>(intent: &T) -> Result<Digest, serde_json::Error> {
    use sha2::{Digest as _, Sha256};
    let mut value = serde_json::to_value(intent)?;
    value.sort_all_objects();
    let bytes = serde_json::to_vec(&value)?;
    let mut hash = Sha256::new();
    hash.update(b"meshway/node-intent/v1\0");
    hash.update(bytes);
    Ok(Digest::from_bytes(hash.finalize().into()))
}
