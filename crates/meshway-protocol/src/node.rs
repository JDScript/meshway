//! Ed25519 challenge authentication. Sign `proof_transcript`, not JSON serialization.
use crate::{WireSecret, object::wire_object};
use meshway_model::*;
use serde::Serialize;
use utoipa::ToSchema;

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Operation bound into the signed Node challenge transcript.
pub enum ProofOperation {
    /// Prove control-key possession to obtain a Node session.
    Session,
    /// Prove old and new control-key possession to rotate Node keys.
    Rotate,
}
}
wire_object! {
/// Identity and operation binding included in a signed challenge application.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeChallengeRequest {
    /// Network to which the registered Node is bound.
    pub network_id: NetworkId,
    /// Registered Node requesting the challenge.
    pub node_id: NodeId,
    /// Current bound control key used to authorize the challenge.
    pub key_id: KeyId,
    /// Operation for which the challenge may be consumed.
    pub operation: ProofOperation,
    /// Domain-separated SHA-256 digest of the canonical typed operation intent.
    pub request_digest: Digest,
} }
wire_object! {
/// Server-issued one-time challenge with exact identity, operation and deadline bindings.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeChallenge {
    /// Client-chosen challenge application identifier retained by the Server.
    pub id: ChallengeId,
    /// Durable Server installation identifier pinned alongside its HTTPS origin.
    pub controller_id: KeyId,
    /// Network bound into the proof transcript.
    pub network_id: NetworkId,
    /// Registered Node bound into the proof transcript.
    pub node_id: NodeId,
    /// Current control key whose signature consumes the challenge.
    pub key_id: KeyId,
    /// Bound key generation checked when the proof is consumed.
    pub key_generation: Version,
    /// Server-generated 32-byte random challenge nonce.
    pub nonce: Digest,
    /// Only operation for which the challenge is valid.
    pub operation: ProofOperation,
    /// Digest of the exact typed intent authorized by this challenge.
    pub request_digest: Digest,
    /// UTC deadline after which the challenge cannot be consumed for a new operation.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! {
/// Signature proving possession of the control key for an exact challenge transcript.
pub struct NodeProof {
    /// Identifier of the challenge signed by this proof.
    pub challenge_id: ChallengeId,
    /// 64-byte Ed25519 signature encoded in unpadded base64url.
    pub signature: WireSecret,
} }
wire_object! {
/// Stable intent used to recover an unexpired Node session after an uncertain response.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeSessionIntent {
    /// Client request identifier reused for the same session intent.
    pub request_id: RequestId,
} }
wire_object! {
/// Session intent and fresh one-time proof submitted by the registered Node.
pub struct NodeSessionRequest {
    /// Session intent whose digest is bound by the challenge.
    pub intent: NodeSessionIntent,
    /// Control-key signature over the challenge transcript.
    pub proof: NodeProof
} }
wire_object! {
/// Short-lived opaque bearer session issued after validating the Node's control-key proof.
pub struct NodeSessionResponse {
    /// Registered Node authenticated by this session.
    pub node_id: NodeId,
    /// Network to which the session is restricted.
    pub network_id: NetworkId,
    /// Control key whose current binding authorized the session.
    pub key_id: KeyId,
    /// Secret bearer credential for subsequent Node requests.
    pub token: WireSecret,
    /// UTC session deadline, preserved when the same unexpired session is recovered.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! {
/// Exact replacement keys bound into a Node key-rotation proof.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeRotationIntent {
    /// Stable identifier reused to recover the same completed rotation.
    pub request_id: RequestId,
    /// New Ed25519 control public key.
    pub control_key: NodePublicKey,
    /// New independent X25519 data-plane public key.
    pub dataplane_key: NodePublicKey,
} }
wire_object! {
/// Rotation request proving possession of both the old and replacement control keys.
pub struct NodeRotationRequest {
    /// Replacement keys and stable rotation request identifier.
    pub intent: NodeRotationIntent,
    /// Existing control key's signature over the rotation challenge.
    pub old_proof: NodeProof,
    /// New control key's 64-byte signature over the same challenge, encoded as unpadded base64url.
    pub new_signature: WireSecret,
} }
wire_object! {
/// Public metadata for the Node's separately bound control and data-plane keys.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeKeyMetadata {
    /// Bound Ed25519 control identity key.
    pub control: BoundNodeKey,
    /// Bound X25519 data-plane key.
    pub dataplane: BoundNodeKey,
} }
wire_object! {
/// Permission-filtered Node registration, configured identity, key metadata and current action eligibility.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeInventoryView {
    /// Stable registered Node identifier.
    pub id: NodeId,
    /// Network to which the Node is registered.
    pub network_id: NetworkId,
    /// UTC time at which the Node was registered.
    #[serde(rename="registeredAt", with="meshway_model::timestamp")]
    pub registered_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Canonical hostname from the current configuration, not the reported OS hostname.
    pub hostname: String,
    /// Whether configuration enables business execution; disabled Nodes may still synchronize control state.
    pub enabled: bool,
    /// Current configured User or Tag authorization identity.
    pub identity: meshway_configuration::NodeIdentity,
    /// Desired Mesh addresses from the current configuration.
    pub addresses: meshway_configuration::NodeAddresses,
    /// Public metadata of the latest bound keys.
    pub keys: NodeKeyMetadata,
    /// Revocation of the displayed latest keys; revoked configured nodes remain manageable.
    #[serde(rename="controlKeyRevokedAt", with="meshway_model::timestamp::optional")]
    pub control_key_revoked_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// UTC revocation time of the displayed data-plane key, or null if it is not revoked.
    #[serde(rename="dataplaneKeyRevokedAt", with="meshway_model::timestamp::optional")]
    pub dataplane_key_revoked_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// Whether the current caller may permanently delete this Node.
    pub can_delete: bool,
    /// Current execution target, not a saved/applied receipt. May be a later refresh than the source apply.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub desired_execution: Option<ConfigurationRef> [required=true],
    /// Whether the current caller may revoke the displayed control key.
    pub can_revoke_control_key: bool,
    /// Whether the current caller may revoke the displayed data-plane key.
    pub can_revoke_dataplane_key: bool,
    /// UTC creation time of the displayed control key binding.
    #[serde(rename="controlKeyCreatedAt", with="meshway_model::timestamp")]
    pub control_key_created_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// UTC creation time of the displayed data-plane key binding.
    #[serde(rename="dataplaneKeyCreatedAt", with="meshway_model::timestamp")]
    pub dataplane_key_created_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! {
/// Registered Nodes visible under the caller's current network permissions.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeInventoryResponse {
    /// Visible Node inventory entries with current action eligibility.
    pub items: Vec<NodeInventoryView>,
} }
wire_object! {
/// Non-secret Node settings projected from one configuration revision.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeSettingsResponse {
    /// Server request identifier for correlation.
    pub request_id: RequestId,
    /// Network containing the settings.
    pub network_id: NetworkId,
    /// Node whose settings are returned.
    pub node_id: NodeId,
    /// Configuration revision from which all settings were read.
    pub revision: Revision,
    /// Non-secret configured settings visible to the authorized caller.
    pub settings: meshway_configuration::NodeSettingsView,
} }
wire_object! {
/// Exact Mesh DNS name lookup in the authenticated Node's visible configuration.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeDnsQueryRequest {
    /// Exact FQDN; raw UTF-8 is bounded before strict DNS/IDNA normalization.
    pub name: String [max_length=1024],
} }
wire_object! {
/// Visible configured Mesh name and addresses for a Node.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeDnsRecord {
    /// Visible Node identified by the queried name.
    pub node_id: NodeId,
    /// Canonical fully qualified Mesh DNS name.
    pub name: String,
    /// Configured Mesh addresses associated with this name.
    pub addresses: meshway_configuration::NodeAddresses,
} }
wire_object! {
/// Permission-filtered name lookup snapshot without an execution grant or DNS TTL.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeDnsQueryResponse {
    /// Server request identifier for correlation.
    pub request_id: RequestId,
    /// Network determined from the authenticated Node session.
    pub network_id: NetworkId,
    /// Authenticated Node whose visibility was used for the lookup.
    pub source_node_id: NodeId,
    /// Configuration revision used to resolve and authorize the name.
    pub configuration_revision: Revision,
    /// UTC time at which the Server evaluated the lookup.
    pub as_of: chrono::DateTime<chrono::Utc>,
    /// Unknown and non-visible names have the same null result. This snapshot
    /// carries no execution grant, TTL, or authoritative DNS negative answer.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub record: Option<NodeDnsRecord> [required=true],
} }
wire_object! {
/// Idempotent permanent Node deletion guarded by the expected configuration revision.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct DeleteNodeRequest {
    /// Request identifier binding retries to the same caller, network and deletion intent.
    pub request_id: RequestId,
    /// Configuration revision required by the compare-and-swap deletion transaction.
    pub expected_revision: Revision,
} }
wire_object! {
/// Permanent revocation of an exact Node key binding.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct RevokeNodeKeyRequest {
    /// Bound control or data-plane key to revoke.
    pub key_id: KeyId,
    /// Key generation required to prevent revoking an unexpected replacement.
    pub expected_generation: Version,
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
