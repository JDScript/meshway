use crate::object::wire_object;
use meshway_configuration::execution::*;
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
    /// Allowed routing profile to select for subsequent demands and DNS execution.
    pub profile: String,
} }
wire_object! {
/// Server-committed routing selection and the execution publication produced by the change.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionResponse {
    /// New selection version required by the next compare-and-swap update.
    pub version: Version,
    /// Profile selected by the accepted request.
    pub profile: String,
    /// Execution publication produced after cancelling the source's prior demands.
    pub publication: PublicationId,
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
/// Server-issued runtime qualification with separate instance and execution deadlines.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceLease {
    /// Acquired Node runtime instance.
    pub instance_id: InstanceId,
    /// Server-issued generation fencing replaced instances.
    pub generation: Version,
    /// Instance deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Execution deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; null means no execution lease is granted.
    #[serde(rename="executionExpiresAt", with="meshway_model::timestamp::optional")]
    pub execution_expires_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// Exact current execution target, or null when no target is available; its presence alone does not grant execution.
    pub target: Option<ConfigurationRef>,
} }
wire_object! {
/// Node request for business execution, with the source and network supplied by its authenticated session.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct DemandRequest {
    /// Current source runtime instance to which the demand is bound.
    pub instance: InstanceRequest,
    /// Stable demand identifier reused with identical content on creation retries.
    pub request_id: RequestId,
    /// Exact requested business destination.
    pub target: BusinessTarget,
    /// Inner business protocol to authorize.
    pub protocol: BusinessProtocol,
    /// TCP or UDP destination port; null or omission is required for ICMP echo.
    pub port: Option<meshway_configuration::transport::Port>,
    /// Authorized routing profile matching the source's current selection.
    pub profile: String,
    /// Requested demand lifetime in seconds, from 1 through 300; creation replay does not extend it.
    pub duration_seconds: u16,
} }
wire_object! {
/// Node request to cancel an existing demand owned by its current instance.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct CancelDemandRequest {
    /// Current source instance required to cancel the demand.
    pub instance: InstanceRequest,
    /// Existing demand identifier to cancel.
    pub request_id: RequestId,
} }
wire_object! {
/// Explicit renewal of a live demand without changing its original business intent.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct RenewDemandRequest {
    /// Current source instance that owns the existing demand.
    pub instance: InstanceRequest,
    /// Existing demand identifier, not a new renewal request identifier.
    pub request_id: RequestId,
    /// Absolute desired deadline of the existing demand; replay never adds duration.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! {
/// Server compilation feedback requiring configuration synchronization before execution.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct DemandResponse {
    /// Execution publication associated with the evaluated demand.
    pub publication: PublicationId,
    /// Current authorization and path result, independent of actual Node installation.
    pub decision: DemandDecision,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Durable save or installation outcome reported by the Node for an exact execution target.
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
/// Node-reported durable result ordered by instance, target, attempt and sequence rather than HTTP arrival.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct ReceiptRequest {
    /// Runtime instance and generation that performed this attempt.
    pub instance: InstanceRequest,
    /// Exact complete target saved or installed by the attempt.
    pub target: ConfigurationRef,
    /// Actual durable save or installation result.
    pub phase: ReceiptPhase,
    /// Increasing application attempt within this instance/target.
    pub attempt: Version,
    /// Increasing result sequence within the attempt; delivery retries reuse it.
    pub sequence: Sequence,
    /// Original Node result time in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and preserved on retry.
    #[serde(rename="reportedAt", with="meshway_model::timestamp")]
    pub reported_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    /// Failed step required for the failed phase; null or omitted for saved and applied.
    pub failure: Option<ApplyFailure>,
} }
wire_object! {
/// Instance-bound request to synchronize a complete execution target or explicitly refresh its lease.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeConfigurationRequest {
    /// Current runtime instance receiving the target.
    pub instance: InstanceRequest,
    /// Exact locally saved base for delta or unchanged detection; null or omission requests a full target when available.
    pub base: Option<ConfigurationRef>,
} }
wire_object! {
/// Consistent Server snapshot of the instance lease and its full, delta or unchanged execution target.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeConfigurationResponse {
    /// Qualification for the exact response target; only explicit renewal operations extend its deadlines.
    pub lease: InstanceLease,
    /// Full replacement or omitted when the exact acknowledged target matches.
    /// A stale base always receives a complete value, never an unverified patch.
    pub full: Option<CompiledNode>,
    /// Delta from a retained matching base; null when returning a full target, an unchanged target or no target.
    pub delta: Option<NodeConfigurationDelta>,
} }
wire_object! {
/// Administrator-visible execution publication targets and Server-derived receipt progress.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct PublicationProgress {
    /// Execution publication whose fixed targets are described.
    pub publication: PublicationId,
    /// Source configuration revision used by the publication.
    pub revision: Revision,
    /// Stored sequence of this execution publication.
    pub sequence: Sequence,
    /// UTC withdrawal time, or null while the publication has not been withdrawn.
    #[serde(rename="withdrawnAt", with="meshway_model::timestamp::optional")]
    pub withdrawn_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    /// Per-Node targets and receipt projections for this publication.
    pub nodes: BTreeMap<NodeId,NodePublicationProgress>,
} }
wire_object! {
/// Server-derived receipt projection for one Node's target within a publication.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct NodePublicationProgress {
    /// Exact target assigned to this Node by the publication.
    pub target: ConfigurationRef,
    /// Whether this publication is still the Node's desired execution publication.
    pub current: bool,
    /// Whether a saved receipt exists for the current instance and generation in this publication.
    pub saved: bool,
    /// Whether freshness is current and the latest attempt and sequence report applied.
    pub applied: bool,
    /// Whether the latest receipt by attempt and sequence for the current instance reports failure.
    pub failed: bool,
    /// Current validity of the target and instance, evaluated independently of receipt phase.
    pub freshness: ReceiptFreshness,
    /// Latest Server reception time among this Node's publication receipts, or null when none exist.
    #[serde(rename="lastReceivedAt", with="meshway_model::timestamp::optional")]
    pub last_received_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Server evaluation of target retirement, instance binding and execution deadlines.
pub enum ReceiptFreshness {
    /// The target and instance bindings match and their applicable deadlines have not elapsed.
    Current,
    /// The matching instance or its execution lease has reached its deadline.
    Expired,
    /// No matching instance context exists for the current Server boot and receipt binding.
    AwaitingInstance,
    /// The target was retired or replaced, its publication withdrawn, or its identity revoked.
    Superseded,
}
}
