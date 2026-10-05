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
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceRequest {
    pub instance_id: InstanceId, pub generation: Version,
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
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct PublicationProgress {
    pub publication: PublicationId, pub revision: Revision, pub sequence: Sequence,
    #[serde(rename="withdrawnAt", with="meshway_model::timestamp::optional")]
    pub withdrawn_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
    pub nodes: BTreeMap<NodeId,NodePublicationProgress>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodePublicationProgress {
    pub target: ConfigurationRef, pub current: bool, pub saved: bool,
    pub applied: bool, pub failed: bool,
    pub freshness: ReceiptFreshness,
    #[serde(rename="lastReceivedAt", with="meshway_model::timestamp::optional")]
    pub last_received_at_ms: Option<u64> [value_type=Option<meshway_model::timestamp::Timestamp>,required=true],
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptFreshness {
    Current,
    Expired,
    AwaitingInstance,
    Superseded,
}
}
