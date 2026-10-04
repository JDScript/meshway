use crate::{WireSecret, node::NodeKeyMetadata, object::wire_object};
use meshway_configuration::transport::Listener;
use meshway_model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InitialNodeSettings {
    pub hostname: Option<String>, pub listeners: BTreeMap<String,Listener>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct CreateEnrollmentKeyRequest {
    pub request_id: RequestId, pub tags: Vec<String>, pub initial_settings: InitialNodeSettings,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentKeyView {
    pub id: EnrollmentKeyId, pub tags: Vec<String>,
    pub created_by: UserId,
    pub initial_settings: InitialNodeSettingsView,
    pub actions: EnrollmentKeyActions,
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub node_id: Option<NodeId> [required=true],
    #[serde(rename="consumedAt", with="meshway_model::timestamp::optional")]
    pub consumed_at_ms: Option<u64> [value_type=Option<chrono::DateTime<chrono::Utc>>,required=true],
    #[serde(rename="createdAt", with="meshway_model::timestamp")]
    pub created_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
    #[serde(rename="revokedAt", with="meshway_model::timestamp::optional")]
    pub revoked_at_ms: Option<u64> [value_type=Option<chrono::DateTime<chrono::Utc>>,required=true],
    pub consumed_request_id: Option<EnrollmentRequestId>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InitialNodeSettingsView {
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub hostname: Option<String> [required=true],
    pub listeners: BTreeMap<String,meshway_configuration::ListenerView>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentKeyActions { pub revoke: bool } }
wire_object! { pub struct EnrollmentKeyResponse {
    pub key: EnrollmentKeyView, pub secret_delivery: SecretDelivery, pub secret: Option<WireSecret>,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SecretDelivery {
    Available,
    Unavailable,
}
wire_object! { pub struct EnrollmentKeysResponse {
    pub items: Vec<EnrollmentKeyView>, pub total: Count,
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub next_cursor: Option<String> [required=true], pub as_of: chrono::DateTime<chrono::Utc>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentIntent {
    pub id: EnrollmentRequestId, pub control_key: NodePublicKey, pub dataplane_key: NodePublicKey,
    pub code_challenge: String, pub hostname: String,
    /// Always present in the signed intent; a first enrollment uses explicit null.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub existing: Option<ExistingNodeBinding> [required=true],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct ExistingNodeBinding {
    pub network_id:NetworkId,pub node_id:NodeId,pub key_id:KeyId,pub key_generation:Version,
} }
wire_object! { pub struct CreateEnrollmentRequest {
    pub intent: EnrollmentIntent, pub signature: WireSecret,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentRequestView {
    pub id: EnrollmentRequestId, pub hostname: String, pub control_key: NodePublicKey,
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub existing: Option<ExistingNodeBinding> [required=true],
    pub return_method: EnrollmentReturnMethod,
    pub phase: EnrollmentPhase,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EnrollmentReturnMethod {
    Poll,
}
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct DeviceAuthorizationView {
    pub request: EnrollmentRequestView,
    /// Only the approving user or a current manager of the approved network sees this fragment.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub approval: Option<EnrollmentApprovalView> [required=true],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentApprovalView {
    pub user_id: UserId, pub network_id: NetworkId,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EnrollmentPhase {
    Pending,
    Approved,
    Withdrawn,
    Cancelled,
    Consumed,
    Expired,
}
wire_object! { pub struct EnrollmentDeviceProof {
    pub verifier: WireSecret, pub signature: WireSecret,
} }
wire_object! { pub struct EnrollmentPollResponse {
    pub request: EnrollmentRequestView, pub code: Option<WireSecret>, pub node_id: Option<NodeId>,
    /// Available only after the device proves possession of this request.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub approval: Option<EnrollmentApprovalView> [required=true],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct ApproveEnrollmentRequest {
    pub network_id: NetworkId,
} }
wire_object! { pub struct RedeemEnrollmentRequest {
    pub proof: EnrollmentDeviceProof, pub code: WireSecret,
} }
wire_object! { pub struct KeyEnrollmentRequest {
    pub enrollment_key: WireSecret, pub request: CreateEnrollmentRequest, pub verifier: WireSecret,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentResponse {
    pub controller_id: KeyId,
    pub network_id: NetworkId, pub node_id: NodeId, pub keys: NodeKeyMetadata,
} }

/// Enrollment intent proof domain is separate from registered-node session proof.
/// The input digest is generated by node-intent/v1 over the typed intent object.
pub fn enrollment_transcript(id: EnrollmentRequestId, digest: Digest, operation: &str) -> Vec<u8> {
    let mut out = b"meshway/enrollment/ed25519/v1\0".to_vec();
    out.extend_from_slice(&(operation.len() as u64).to_be_bytes());
    out.extend_from_slice(operation.as_bytes());
    out.extend_from_slice(id.as_uuid().as_bytes());
    out.extend_from_slice(digest.as_bytes());
    out
}
