use crate::{WireSecret, node::NodeKeyMetadata, object::wire_object};
use meshway_configuration::transport::Listener;
use meshway_model::*;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

wire_object! {
/// Initial configuration to validate and apply when an enrollment key registers a Node.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InitialNodeSettings {
    /// Requested hostname, or null to use the enrolling device's hostname.
    pub hostname: Option<String>,
    /// Initial listeners indexed by configuration key, including supplied transport material.
    pub listeners: BTreeMap<String,Listener>,
} }
wire_object! {
/// Administrator request for a one-time, expiring Tag enrollment key.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct CreateEnrollmentKeyRequest {
    /// Idempotency identifier bound to the caller, network and exact creation request.
    pub request_id: RequestId,
    /// Preassigned Tags that define the enrolled Node's authorization identity.
    pub tags: Vec<String>,
    /// Initial Node settings validated before the key is created.
    pub initial_settings: InitialNodeSettings,
    /// UTC deadline after which the key cannot enroll a Node.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! {
/// Non-secret enrollment key metadata visible to authorized network administrators.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentKeyView {
    /// Stable identifier of the enrollment key record.
    pub id: EnrollmentKeyId,
    /// Tags assigned to the Node when this key is consumed.
    pub tags: Vec<String>,
    /// User who created the enrollment key.
    pub created_by: UserId,
    /// Non-secret summary of the initial Node configuration.
    pub initial_settings: InitialNodeSettingsView,
    /// Current caller's eligibility to operate on this key.
    pub actions: EnrollmentKeyActions,
    /// Node registered by this key, or null before consumption.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub node_id: Option<NodeId> [required=true],
    /// UTC consumption time, or null if the key has not been consumed.
    #[serde(rename="consumedAt", with="meshway_model::timestamp::optional")]
    pub consumed_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// UTC time at which the key was created.
    #[serde(rename="createdAt", with="meshway_model::timestamp")]
    pub created_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// UTC deadline after which the key cannot be consumed.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// UTC revocation time, or null if the key has not been revoked.
    #[serde(rename="revokedAt", with="meshway_model::timestamp::optional")]
    pub revoked_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// Enrollment request that consumed this key, or null before consumption.
    pub consumed_request_id: Option<EnrollmentRequestId>,
} }
wire_object! {
/// Non-secret initial settings retained with an enrollment key.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InitialNodeSettingsView {
    /// Requested hostname, or null when the device supplies it at enrollment.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub hostname: Option<String> [required=true],
    /// Listener summaries excluding passwords, private keys and header values.
    pub listeners: BTreeMap<String,meshway_configuration::ListenerView>,
} }
wire_object! {
/// Current caller's permitted enrollment key operations, rechecked on mutation.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentKeyActions {
    /// Whether the caller may currently revoke this key.
    pub revoke: bool
} }
wire_object! {
/// Enrollment key creation result with one-time secret delivery.
pub struct EnrollmentKeyResponse {
    /// Created key's non-secret metadata, also returned on an idempotent retry.
    pub key: EnrollmentKeyView,
    /// Whether this response includes the newly created secret.
    pub secret_delivery: SecretDelivery,
    /// Raw enrollment credential on first delivery, or null when delivery is unavailable.
    pub secret: Option<WireSecret>,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Availability of a creation secret in this response.
pub enum SecretDelivery {
    /// The first successful response includes the newly created secret.
    Available,
    /// Metadata is returned without the secret, including idempotent creation retries.
    Unavailable,
}
}
wire_object! {
/// Newest-first enrollment key page under the caller's current network permissions.
pub struct EnrollmentKeysResponse {
    /// Non-secret enrollment key records in this page.
    pub items: Vec<EnrollmentKeyView>,
    /// Number of matching keys within the initial page's creation upper bound.
    pub total: Count,
    /// Continuation cursor for the same bounded listing, or null on the final page.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub next_cursor: Option<String> [required=true],
    /// UTC snapshot time retained across continuation pages.
    pub as_of: chrono::DateTime<chrono::Utc>,
} }
wire_object! {
/// Signed device enrollment intent binding public keys, PKCE and any existing registration.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentIntent {
    /// Device-generated identifier for this enrollment attempt.
    pub id: EnrollmentRequestId,
    /// Device's persistent Ed25519 control public key.
    pub control_key: NodePublicKey,
    /// Independent X25519 public key for data-plane protection.
    pub dataplane_key: NodePublicKey,
    /// PKCE challenge binding later redemption to the device's private verifier.
    pub code_challenge: String,
    /// Device-provided hostname used as the initial naming input.
    pub hostname: String,
    /// Always present in the signed intent; a first enrollment uses explicit null.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub existing: Option<ExistingNodeBinding> [required=true],
} }
wire_object! {
/// Current registered identity that a device must prove when enrolling again.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct ExistingNodeBinding {
    /// Existing network binding, which approval cannot replace with another network.
    pub network_id:NetworkId,
    /// Existing Node identity retained by re-enrollment.
    pub node_id:NodeId,
    /// Currently bound control key that must match the enrollment public key.
    pub key_id:KeyId,
    /// Current control-key generation required by the existing binding.
    pub key_generation:Version,
} }
wire_object! {
/// Device request to begin enrollment with proof of control-key possession.
pub struct CreateEnrollmentRequest {
    /// Exact enrollment intent covered by the device signature.
    pub intent: EnrollmentIntent,
    /// Control-key signature over the enrollment transcript, encoded as unpadded base64url.
    pub signature: WireSecret,
} }
wire_object! {
/// Public device enrollment request details and its current lifecycle phase.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentRequestView {
    /// Identifier of the device's enrollment request.
    pub id: EnrollmentRequestId,
    /// Hostname supplied by the requesting device.
    pub hostname: String,
    /// Public control identity whose possession the device proved.
    pub control_key: NodePublicKey,
    /// Existing registration being reauthorized, or null for first enrollment.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub existing: Option<ExistingNodeBinding> [required=true],
    /// Mechanism by which the device retrieves the authorization outcome.
    pub return_method: EnrollmentReturnMethod,
    /// Current Server-evaluated request lifecycle phase.
    pub phase: EnrollmentPhase,
    /// UTC deadline for completing this enrollment request.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Where a signed-in user authorizes this device, built by the Server from
    /// its Console origin; null when the deployment has not configured one.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub authorization_url: Option<String> [required=true],
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Supported mechanism for delivering an enrollment outcome to the device.
pub enum EnrollmentReturnMethod {
    /// The device queries the request using its possession proof.
    Poll,
}
}
wire_object! {
/// Signed-in user's view of a device authorization, with approval details filtered by current permissions.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct DeviceAuthorizationView {
    /// Device request and its current lifecycle state.
    pub request: EnrollmentRequestView,
    /// Only the approving user or a current manager of the approved network sees this fragment.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub approval: Option<EnrollmentApprovalView> [required=true],
} }
wire_object! {
/// Approving user and selected network for a device enrollment.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentApprovalView {
    /// User whose approval authorizes the device enrollment.
    pub user_id: UserId,
    /// Network selected by the approving user and rechecked at redemption.
    pub network_id: NetworkId,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Server-evaluated lifecycle of a device enrollment request.
pub enum EnrollmentPhase {
    /// The device request is awaiting user approval.
    Pending,
    /// A user approved enrollment and the device has not yet redeemed it.
    Approved,
    /// The approving user withdrew approval before redemption.
    Withdrawn,
    /// The device cancelled the request with a valid possession proof.
    Cancelled,
    /// The enrollment result was redeemed and the Node binding committed.
    Consumed,
    /// The request reached its enrollment deadline.
    Expired,
}
}
wire_object! {
/// Device possession proof bound to the original enrollment request and PKCE challenge.
pub struct EnrollmentDeviceProof {
    /// Private PKCE verifier matching the original intent's challenge.
    pub verifier: WireSecret,
    /// Device control-key signature for the requested enrollment operation.
    pub signature: WireSecret,
} }
wire_object! {
/// Enrollment outcome delivered only after validating the requesting device's proof.
pub struct EnrollmentPollResponse {
    /// Enrollment request's current metadata and phase.
    pub request: EnrollmentRequestView,
    /// One-time redemption code when an approved result is deliverable, or null otherwise.
    pub code: Option<WireSecret>,
    /// Registered Node after consumption, or null before the registration result exists.
    pub node_id: Option<NodeId>,
    /// Available only after the device proves possession of this request.
    #[serde(deserialize_with="crate::object::required_nullable")]
    pub approval: Option<EnrollmentApprovalView> [required=true],
} }
wire_object! {
/// User approval selecting an eligible network for the device.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct ApproveEnrollmentRequest {
    /// Eligible network in which to register or reauthorize the device.
    pub network_id: NetworkId,
} }
wire_object! {
/// Device redemption of a user-approved enrollment result.
pub struct RedeemEnrollmentRequest {
    /// Device signature and PKCE verifier proving ownership of the enrollment request.
    pub proof: EnrollmentDeviceProof,
    /// One-time approval result delivered through the device's authenticated query.
    pub code: WireSecret,
} }
wire_object! {
/// Non-interactive registration using a one-time Tag enrollment key and device key proof.
pub struct KeyEnrollmentRequest {
    /// Secret one-time enrollment credential for the target network.
    pub enrollment_key: WireSecret,
    /// Signed device enrollment intent binding its own public keys.
    pub request: CreateEnrollmentRequest,
    /// PKCE verifier matching the challenge in the signed intent.
    pub verifier: WireSecret,
} }
wire_object! {
/// Committed Node registration and public key bindings returned to the enrolling device.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct EnrollmentResponse {
    /// Durable Server installation identity to pin alongside the HTTPS origin.
    pub controller_id: KeyId,
    /// Network to which the device is now registered.
    pub network_id: NetworkId,
    /// Stable registered Node identity.
    pub node_id: NodeId,
    /// Public control and data-plane key bindings committed by enrollment.
    pub keys: NodeKeyMetadata,
    /// Server key signing netmap leases, pinned by the Node with this binding; null or omission means the Server issues no signed netmaps and the Node accepts none.
    pub server_signing_key: Option<crate::netmap::ServerSigningKey>,
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
