//! Source facts. These values neither grant authority nor contain transport secrets.
pub use crate::ConfigurationRef;
use crate::{Digest, EventId, NodeId, ObservationId, PublicationId, Sequence};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{collections::BTreeMap, net::SocketAddr};
use utoipa::ToSchema;

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct SystemState {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransportRole {
    Dial,
    Listen,
}
}

meshway_serde::tagged! {
tag = "kind";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    Transport {
        roles: Vec<TransportRole>,
        streams: Vec<StreamKind>,
    },
    Feature {
        version: u16,
    },
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    Tcp,
    Websocket,
    Udp,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApplyPhase {
    Receiving,
    Validating,
    Saved,
    Applying,
    Applied,
    Failed,
}
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ConfigurationProgress {
    pub target: ConfigurationRef,
    pub phase: ApplyPhase,
    pub observed_at: DateTime<Utc>,
    pub problem: Option<StateProblem>,
} }

meshway_serde::object! { #[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct ConfigurationState {
    pub saved: Option<ConfigurationRef>,
    pub applied: Option<ConfigurationRef>,
    pub progress: Option<ConfigurationProgress>,
} }

meshway_serde::tagged! { tag="kind"; #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] pub enum ResourceRef {
    Listener { key: String },
    Dial { key: String },
    Coordination { key: String },
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SocketNetwork {
    Udp,
    Tcp,
}
}

meshway_serde::tagged! { tag="source"; #[derive(Clone, Debug, PartialEq, Eq)] pub enum EndpointSource {
    Configured {
        #[serde(rename = "configPath")]
        config_path: String,
    },
    Interface {
        interface: String,
    },
    Stun {
        observer: String,
    },
} }

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct EndpointCandidate {
    pub address: String,
    pub origin: EndpointSource,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
} }

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct SocketState {
    pub resource: ResourceRef,
    /// None inherits the report's applied configuration; installation sockets
    /// must name their target even before the first configuration is applied.
    pub configuration: Option<ConfigurationRef>,
    pub generation: Sequence,
    pub network: SocketNetwork,
    pub bound_address: SocketAddr [value_type = String],
    pub endpoints: Vec<EndpointCandidate>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ListenerPhase {
    Preparing,
    Listening,
    Stopped,
    Failed,
}
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ListenerState {
    pub phase: ListenerPhase,
    pub socket_ids: Vec<String>,
    pub loaded_certificate_sha256: Option<Digest>,
    pub problem: Option<StateProblem>,
    pub configuration: Option<ConfigurationRef>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkPhase {
    Preparing,
    Authenticated,
    Closed,
    Failed,
}
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct LinkState {
    pub link_id: String,
    pub transport: String,
    pub initiator_node_id: NodeId,
    pub socket_id: String,
    pub phase: LinkPhase,
    pub business_dependencies: u32,
    pub observation_ids: Vec<ObservationId>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub since: Option<DateTime<Utc>>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReporterRole {
    Origin,
    Relay,
    Target,
    Exit,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PathEvidence {
    Selected,
    Sent,
    PeerConfirmed,
    Delivered,
}
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct PathState {
    pub nodes: Vec<NodeId>,
    pub publication_id: PublicationId,
    pub reporter_role: ReporterRole,
    pub evidence: PathEvidence,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProbeKind {
    Neighbor,
    Path,
    Connect,
    Controller,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProbeOutcome {
    Pending,
    Success,
    Timeout,
    Failed,
}
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeObservation {
    pub measurement_id: EventId,
    pub kind: ProbeKind,
    pub link_id: Option<String>,
    pub transport: Option<String>,
    pub path: Vec<NodeId>,
    pub publication_id: PublicationId,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub outcome: ProbeOutcome,
    pub duration_us: Option<Sequence>,
} }

meshway_serde::tagged! {
tag = "code";
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateProblem {
    ResourceUnavailable = "resource_unavailable" {
        resource: ResourceRef,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    ConfigurationRejected = "configuration_rejected" {
        target: ConfigurationRef,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    ProbeFailed = "probe_failed" {
        #[serde(rename = "measurementId")]
        measurement_id: EventId,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
}
}

meshway_serde::object! { #[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct SelectionState {
    pub applied_profile: Option<String>,
} }

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeState {
    pub system: SystemState,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub capabilities: BTreeMap<String, Capability>,
    pub configuration: ConfigurationState,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub listeners: BTreeMap<String, ListenerState>,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub sockets: BTreeMap<String, SocketState>,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub links: BTreeMap<NodeId, Vec<LinkState>>,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub paths: BTreeMap<String, PathState>,
    pub selection: SelectionState,
    pub probes: Vec<ProbeObservation>,
    pub problems: Vec<StateProblem>,
} }
