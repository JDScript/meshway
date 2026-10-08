use crate::object::wire_object;
use meshway_configuration::execution::*;
use meshway_model::*;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionRequest {
    pub instance: InstanceRequest, pub expected_version: Option<Version>, pub profile: String,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionResponse {
    pub version: Version, pub profile: String, pub publication: PublicationId,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct AcquireInstanceRequest {
    pub instance_id: InstanceId, pub capabilities: BTreeMap<String,meshway_model::state::Capability>,
} }
wire_object! {
/// Exact runtime instance qualification required for an instance-bound operation.
#[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceRequest {
    /// Instance identifier acquired for the current Node runtime.
    pub instance_id: InstanceId,
    /// Server-issued instance generation that rejects superseded callers.
    pub generation: Version,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceLease {
    pub instance_id: InstanceId, pub generation: Version,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    #[serde(rename="executionExpiresAt", with="meshway_model::timestamp::optional")]
    pub execution_expires_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    pub target: Option<ConfigurationRef>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct DemandRequest {
    pub instance: InstanceRequest, pub request_id: RequestId,
    pub target: BusinessTarget, pub protocol: BusinessProtocol,
    pub port: Option<meshway_configuration::transport::Port>, pub profile: String,
    pub duration_seconds: u16,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct CancelDemandRequest {
    pub instance: InstanceRequest, pub request_id: RequestId,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RenewDemandRequest {
    pub instance: InstanceRequest, pub request_id: RequestId,
    /// Absolute desired deadline of the existing demand; replay never adds duration.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct DemandResponse {
    pub publication: PublicationId, pub decision: DemandDecision,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptPhase {
    Saved,
    Applied,
    Failed,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApplyFailure {
    Validation,
    Storage,
    Listeners,
    Underlay,
    Tun,
    Dns,
    Dataplane,
}
}
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct ReceiptRequest {
    pub instance: InstanceRequest, pub target: ConfigurationRef, pub phase: ReceiptPhase,
    /// Increasing application attempt within this instance/target.
    pub attempt: Version,
    /// Increasing result sequence within the attempt; delivery retries reuse it.
    pub sequence: Sequence,
    #[serde(rename="reportedAt", with="meshway_model::timestamp")]
    pub reported_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
    pub failure: Option<ApplyFailure>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeConfigurationRequest {
    pub instance: InstanceRequest, pub base: Option<ConfigurationRef>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodeConfigurationResponse {
    pub lease: InstanceLease,
    /// Full replacement or omitted when the exact acknowledged target matches.
    /// A stale base always receives a complete value, never an unverified patch.
    pub full: Option<CompiledNode>,
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
