use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{
    InstanceId, NetworkId, NodeId, Revision, Sequence, StateEpoch,
    resources::{GeoObservation, IpInfo, LocationAbsentReason},
    state::{Capability, ConfigurationState, NodeState, ResourceRef, SocketNetwork},
};
use std::collections::BTreeMap;

/// A State report is accepted only within this many seconds of its
/// `observedAt`; an undelivered report older than this is never retried.
pub const STATE_REPORT_TTL_SECONDS: i64 = 60;
/// Longest lifetime (`expiresAt - observedAt`) of one reported State fact.
pub const STATE_FACT_MAX_SECONDS: i64 = 600;

wire_object! {
/// Complete Node snapshot authenticated and validated against the current instance.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct StateReport {
    /// Current authenticated runtime instance that produced the snapshot.
    pub instance_id: InstanceId,
    /// Increasing report sequence; identical retries reuse the sequence and content.
    pub sequence: Sequence,
    /// Original UTC time at which the Node observed the snapshot.
    pub observed_at: DateTime<Utc>,
    /// Complete runtime inventory replacing the previous accepted snapshot.
    pub state: NodeState,
} }
wire_object! {
/// Acceptance of a State report, not an execution lease or application receipt.
#[derive(Clone, Debug)] pub struct StateAccepted {
    /// Accepted report sequence.
    pub sequence: Sequence,
    /// Original Server reception time, preserved for identical retries.
    pub received_at: DateTime<Utc>,
    /// Whether the same sequence and content had already been accepted.
    pub duplicate: bool,
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
/// Server-derived validity of the latest report under current identity, instance and configuration state.
pub enum Freshness {
    /// The report is within its TTL and matches the live instance and current execution context.
    Current,
    /// No report has been accepted for the Node.
    AwaitingReport,
    /// The report exceeded its TTL or the Node has no live instance.
    Expired,
    /// A different live instance has replaced the reporting instance.
    Superseded,
    /// The live instance's reported applied target is outside the current configuration set.
    ConfigMismatch,
    /// The Node's credentials have been permanently revoked.
    Revoked,
}
}

wire_object! {
/// Server-derived IP metadata restricted to endpoint observations visible to the caller.
#[derive(Clone, Debug, PartialEq)] pub struct DerivedNodeState {
    /// Independent GeoIP and ASN lookups indexed by canonical visible endpoint IP.
    #[serde(deserialize_with="meshway_model::wire::unique_map")] pub ip_info: BTreeMap<String, IpInfo>,
    /// Approximate location from valid eligible local underlay observations, or null when unavailable.
    pub location: Option<GeoObservation>,
    /// Reason no location can be derived, or null when location is available.
    pub location_absent: Option<LocationAbsentReason>,
} }
wire_object! {
/// Authorized peer endpoint projection retaining its exact runtime resource binding.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerEndpoint {
    /// Socket identifier within the reporting instance.
    pub socket_id: String,
    /// Resource owning the reported socket.
    pub resource: ResourceRef,
    /// Socket resource generation used to reject stale bindings.
    pub generation: Sequence,
    /// Actual outer socket protocol.
    pub network: SocketNetwork,
    /// Visible candidate with source details reduced to their provenance kind.
    pub candidate: PeerCandidate,
    /// Exact candidate binding used by coordinated connection acquisition.
    pub candidate_reference: crate::connectivity::CandidateReference,
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Candidate provenance exposed to peers without private interface or observer details.
pub enum CandidateSource {
    /// The address was explicitly declared in configuration.
    Configured,
    /// The Node observed the address on a local interface.
    Interface,
    /// STUN observed a mapping for the actual UDP socket.
    Stun,
}
}
wire_object! {
/// Peer-visible endpoint candidate with its original observation lifetime.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerCandidate {
    /// Candidate host and port, without asserting authenticated reachability.
    pub address: String,
    /// Coarse provenance of the candidate.
    pub source: CandidateSource,
    /// Original UTC observation time retained from the Node report.
    pub observed_at: DateTime<Utc>,
    /// UTC deadline after which the candidate observation is no longer current.
    pub expires_at: DateTime<Utc>,
} }
// Peer projection deliberately omits interfaces, system details, unrelated paths,
// problems and local bind addresses. It is not the source report DTO.
wire_object! {
/// Restricted peer facts needed for authorized execution dependencies.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PeerState {
    /// Reported build capabilities, empty when the source report is not current.
    #[serde(deserialize_with="meshway_model::wire::unique_map")] pub capabilities: BTreeMap<String, Capability>,
    /// Peer-reported saved, applied and progress references.
    pub configuration: ConfigurationState,
    /// Currently visible connection candidates with their runtime bindings.
    pub endpoints: Vec<PeerEndpoint>,
} }
crate::object::wire_tagged! { tag="view";
/// Report projection selected by the Server under the caller's current permissions.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum VisibleReport {
    /// The caller may read the complete Node report.
    Full {
        /// Complete accepted runtime snapshot.
        report: Box<NodeState>
    },
    /// The caller may read only execution-related peer facts.
    Peer {
        /// Restricted peer snapshot without local system or interface details.
        report: Box<PeerState>
    },
} }
wire_object! {
/// Latest visible Node facts with independently derived freshness and IP information.
#[derive(Clone, Debug, PartialEq)] pub struct NodeStateView {
    /// The configured hostname (display name), not the OS hostname a report carries.
    pub hostname: String,
    /// Instance that produced the accepted report, or null before any report.
    pub instance_id: Option<InstanceId>,
    /// Accepted report sequence, or null before any report.
    pub report_sequence: Option<Sequence>,
    /// Original Node observation time, or null before any report.
    pub observed_at: Option<DateTime<Utc>>,
    /// Original Server reception time, or null before any report.
    pub received_at: Option<DateTime<Utc>>,
    /// UTC deadline of the accepted report's freshness, or null before any report.
    pub report_expires_at: Option<DateTime<Utc>>,
    /// Latest authenticated State-report contact, or null before any accepted report.
    pub last_authenticated_at: Option<DateTime<Utc>>,
    /// Current Server evaluation of the report's identity, instance, TTL and configuration binding.
    pub freshness: Freshness,
    /// Permission-filtered accepted report, or null before any report.
    pub report: Option<VisibleReport>,
    /// IP metadata and approximate location derived by the Server from visible observations.
    pub derived: DerivedNodeState,
} }
wire_object! {
/// Permission-filtered network snapshot assembled from independently timed Node reports.
#[derive(Clone, Debug, PartialEq)] pub struct StateView {
    /// Network whose visible State is returned.
    pub network_id: NetworkId,
    /// Server State generation, replaced after a Server restart.
    pub state_epoch: StateEpoch,
    /// State change sequence within the Server epoch.
    pub sequence: Sequence,
    /// Current configuration used to interpret and filter the view.
    pub configuration_revision: Revision,
    /// UTC time at which the Server evaluated this view.
    pub as_of: DateTime<Utc>,
    /// Visible Node reports indexed by Node identity.
    #[serde(deserialize_with="meshway_model::wire::unique_map")] pub nodes: BTreeMap<NodeId, NodeStateView>,
    /// Channel evidence aggregated from visible reports.
    pub links: Vec<AggregatedLink>,
    /// Path observations retaining each original reporter's evidence.
    pub paths: Vec<AggregatedPath>,
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Server comparison of endpoint reports for the same physical channel.
pub enum LinkEvidence {
    /// Only one endpoint supplies current channel evidence.
    OneSided,
    /// Both endpoints agree on the channel, identities, transport and execution context.
    Matched,
    /// Endpoint reports disagree and cannot be combined as healthy evidence.
    Conflicting,
    /// The available channel evidence is no longer current.
    Expired,
}
}
wire_object! {
/// Exact accepted Node snapshot contributing an aggregate fact.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ReportRef {
    /// Node that produced the original report.
    pub node_id: NodeId,
    /// Runtime instance that produced the report.
    pub instance_id: InstanceId,
    /// Sequence of the contributing report within that instance.
    pub report_sequence: Sequence,
} }
wire_object! {
/// Visible endpoint evidence for one channel, without asserting bidirectional business access.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct AggregatedLink {
    /// First Node in the channel's endpoint pair.
    pub a: NodeId,
    /// Second Node in the channel's endpoint pair.
    pub b: NodeId,
    /// Actual channel identifier shared by matching reports.
    pub link_id: String,
    /// Reported transport used by the channel.
    pub transport: String,
    /// Node that initiated channel establishment.
    pub initiator_node_id: NodeId,
    /// Server evaluation of agreement and freshness across endpoint reports.
    pub evidence: LinkEvidence,
    /// Channel lifecycle phase retained from the reported evidence.
    pub phase: meshway_model::state::LinkPhase,
    /// Original Node snapshots contributing this aggregate.
    pub reports: Vec<ReportRef>,
    /// Whether current evidence reports business dependencies, or null when evidence is insufficient.
    pub in_use: Option<bool>,
    /// Known UTC start of the channel state, or null when evidence is insufficient.
    pub since: Option<DateTime<Utc>>,
} }
wire_object! {
/// One visible path observation with its original reporter and execution context.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct AggregatedPath {
    /// Runtime path identifier from the original Node report.
    pub path_id: String,
    /// Exact Node snapshot that supplied the path observation.
    pub reporter: ReportRef,
    /// Original ordered path, reporter role, evidence stage and validity interval.
    pub state: meshway_model::state::PathState,
} }
wire_object! {
/// Optional subset filter that can only narrow the caller's authorized State view.
#[derive(Clone, Debug)] pub struct StateQuery {
    /// Requested subset; null or omission uses all visible Nodes, while an empty list returns none.
    pub nodes: Option<Vec<NodeId>>,
} }
