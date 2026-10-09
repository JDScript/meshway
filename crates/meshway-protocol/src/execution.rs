use crate::object::wire_object;
use meshway_model::*;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;
wire_object! {
/// Node request to select an authorized routing profile using compare-and-swap against its current selection.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionRequest {
    /// Current runtime instance authorized to change the selection.
    pub instance: InstanceRequest,
    /// Existing selection version to replace; null or omission requires that no selection record exists.
    pub expected_version: Option<Version>,
    /// Allowed routing profile to select for subsequent flows and DNS execution.
    pub profile: String,
} }
wire_object! {
/// Server-committed routing selection; the Node's next netmap carries it.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionResponse {
    /// New selection version required by the next compare-and-swap update.
    pub version: Version,
    /// Profile selected by the accepted request.
    pub profile: String,
} }
wire_object! {
/// Authenticated Node request to acquire its runtime instance with actual build capabilities.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct AcquireInstanceRequest {
    /// Node-generated identifier reused only for an uncertain acquisition of the same still-valid instance.
    pub instance_id: InstanceId,
    /// Actual supported capabilities fixed by authenticated acquisition rather than ordinary State reports.
    pub capabilities: BTreeMap<String,meshway_model::state::Capability>,
} }
wire_object! {
/// Exact runtime instance qualification required for an instance-bound operation.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceRequest {
    /// Instance identifier acquired for the current Node runtime.
    pub instance_id: InstanceId,
    /// Server-issued instance generation that rejects superseded callers.
    pub generation: Version,
} }
wire_object! {
/// Server-issued runtime qualification; it bounds State, receipts and coordination, never execution.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceLease {
    /// Acquired Node runtime instance.
    pub instance_id: InstanceId,
    /// Server-issued generation fencing replaced instances.
    pub generation: Version,
    /// Instance deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Durable save or installation outcome reported by the Node for an exact netmap.
pub enum ReceiptPhase {
    /// The complete validated target was privately persisted.
    Saved,
    /// All required local resources were installed for this target.
    Applied,
    /// A specific save or installation step failed.
    Failed,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Node-reported step responsible for a failed configuration attempt.
pub enum ApplyFailure {
    /// Complete target validation failed.
    Validation,
    /// Private persistence of the target failed.
    Storage,
    /// Required listener installation failed.
    Listeners,
    /// Required underlay resources or socket protection failed.
    Underlay,
    /// Required TUN installation failed.
    Tun,
    /// Required DNS installation or execution setup failed.
    Dns,
    /// Required data-plane installation failed.
    Dataplane,
}
}
wire_object! {
/// Node-reported durable result ordered by instance, netmap, attempt and sequence rather than HTTP arrival.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct ReceiptRequest {
    /// Runtime instance and generation that performed this attempt.
    pub instance: InstanceRequest,
    /// Exact netmap saved or installed by the attempt.
    pub netmap: NetmapRef,
    /// Actual durable save or installation result.
    pub phase: ReceiptPhase,
    /// Increasing application attempt within this instance/netmap.
    pub attempt: Version,
    /// Increasing result sequence within the attempt; delivery retries reuse it.
    pub sequence: Sequence,
    /// Original Node result time in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and preserved on retry.
    #[serde(rename="reportedAt", with="meshway_model::timestamp")]
    pub reported_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Failed step required for the failed phase; null or omitted for saved and applied.
    pub failure: Option<ApplyFailure>,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Server evaluation of netmap retirement and instance binding.
pub enum ReceiptFreshness {
    /// The netmap is the Node's current one and the receipt's instance is live.
    Current,
    /// The receipt's instance has reached its deadline.
    Expired,
    /// No matching instance context exists for the current Server boot and receipt binding.
    AwaitingInstance,
    /// The netmap was replaced by a newer serial or its identity was revoked.
    Superseded,
}
}
