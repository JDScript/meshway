use crate::object::wire_object;
use meshway_configuration::execution::*;
use meshway_model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionRequest {
    pub instance: InstanceRequest, pub expected_version: Option<Version>, pub profile: String,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct RoutingSelectionResponse {
    pub version: Version, pub profile: String, pub publication: PublicationId,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct AcquireInstanceRequest {
    pub instance_id: InstanceId, pub capabilities: BTreeMap<String,TransportCapability>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceRequest {
    pub instance_id: InstanceId, pub generation: Version,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct InstanceLease {
    pub instance_id: InstanceId, pub generation: Version, pub expires_at_ms: u64,
    pub execution_expires_at_ms: Option<u64>, pub target: Option<ConfigurationRef>,
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
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct DemandResponse {
    pub publication: PublicationId, pub decision: DemandDecision,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptPhase {
    Saved,
    Applied,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct ReceiptRequest {
    pub instance: InstanceRequest, pub target: ConfigurationRef, pub phase: ReceiptPhase,
    pub reported_at_ms: u64, pub failure: Option<ApplyFailure>,
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
    pub withdrawn_at_ms: Option<u64>, pub nodes: BTreeMap<NodeId,NodePublicationProgress>,
} }
wire_object! { #[derive(Clone,Debug,PartialEq,Eq)] pub struct NodePublicationProgress {
    pub target: ConfigurationRef, pub current: bool, pub saved: bool,
    pub applied: bool, pub failed: bool,
    pub freshness: ReceiptFreshness, pub last_received_at_ms: Option<u64>,
} }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptFreshness {
    Current,
    Expired,
    AwaitingInstance,
    Superseded,
}
