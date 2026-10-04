use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{
    InstanceId, NetworkId, NodeId, Revision, Sequence, StateEpoch,
    resources::{GeoObservation, IpInfo, LocationAbsentReason},
    state::{Capability, ConfigurationState, NodeState, ResourceRef, SocketNetwork},
};
use std::collections::BTreeMap;

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct StateReport {
    pub instance_id: InstanceId, pub sequence: Sequence, pub observed_at: DateTime<Utc>, pub state: NodeState,
} }
wire_object! { #[derive(Clone, Debug)] pub struct StateAccepted {
    pub sequence: Sequence, pub received_at: DateTime<Utc>, pub duplicate: bool,
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Current,
    AwaitingReport,
    Expired,
    Superseded,
    ConfigMismatch,
    Revoked,
}

wire_object! { #[derive(Clone, Debug, PartialEq)] pub struct DerivedNodeState {
    #[serde(deserialize_with="meshway_serde::unique_map")] pub ip_info: BTreeMap<String, IpInfo>, pub location: Option<GeoObservation>, pub location_absent: Option<LocationAbsentReason>,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerEndpoint {
    pub socket_id: String, pub resource: ResourceRef, pub generation: Sequence,
    pub network: SocketNetwork, pub candidate: PeerCandidate,
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum CandidateSource {
    Configured,
    Interface,
    Stun,
}
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerCandidate {
    pub address: String, pub source: CandidateSource, pub observed_at: DateTime<Utc>, pub expires_at: DateTime<Utc>,
} }
// Peer projection deliberately omits interfaces, system details, unrelated paths,
// problems and local bind addresses. It is not the source report DTO.
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerState {
    #[serde(deserialize_with="meshway_serde::unique_map")] pub capabilities: BTreeMap<String, Capability>, pub configuration: ConfigurationState,
    pub endpoints: Vec<PeerEndpoint>,
} }
crate::object::wire_tagged! { tag="view"; #[derive(Clone, Debug, PartialEq, Eq)] pub enum VisibleReport {
    Full { report: Box<NodeState> },
    Peer { report: Box<PeerState> },
} }
wire_object! { #[derive(Clone, Debug, PartialEq)] pub struct NodeStateView {
    /// The configured hostname (display name), not the OS hostname a report carries.
    pub hostname: String,
    pub instance_id: Option<InstanceId>, pub report_sequence: Option<Sequence>,
    pub observed_at: Option<DateTime<Utc>>,
    pub received_at: Option<DateTime<Utc>>, pub report_expires_at: Option<DateTime<Utc>>,
    pub last_authenticated_at: Option<DateTime<Utc>>, pub freshness: Freshness,
    pub report: Option<VisibleReport>, pub derived: DerivedNodeState,
} }
wire_object! { #[derive(Clone, Debug, PartialEq)] pub struct StateView {
    pub network_id: NetworkId, pub state_epoch: StateEpoch, pub sequence: Sequence,
    pub configuration_revision: Revision, pub as_of: DateTime<Utc>, #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, NodeStateView>,
    pub links: Vec<AggregatedLink>, pub paths: Vec<AggregatedPath>,
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum LinkEvidence {
    OneSided,
    Matched,
    Conflicting,
    Expired,
}
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ReportRef {
    pub node_id: NodeId, pub instance_id: InstanceId, pub report_sequence: Sequence,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct AggregatedLink {
    pub a: NodeId, pub b: NodeId, pub link_id: String, pub transport: String,
    pub initiator_node_id: NodeId, pub evidence: LinkEvidence,
    pub phase: meshway_model::state::LinkPhase,
    pub reports: Vec<ReportRef>, pub in_use: Option<bool>, pub since: Option<DateTime<Utc>>,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct AggregatedPath {
    pub path_id: String, pub reporter: ReportRef, pub state: meshway_model::state::PathState,
} }
wire_object! { #[derive(Clone, Debug)] pub struct StateQuery {
    pub nodes: Option<Vec<NodeId>>,
} }
