//! Source facts. These values neither grant authority nor contain transport secrets.
use crate::{Digest, EventId, NodeId, ObservationId, PublicationId, Revision, Sequence};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationRef {
    pub revision: Revision,
    pub execution_publication: PublicationId,
    pub digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemState {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransportRole {
    Dial,
    Listen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Capability {
    Transport {
        roles: Vec<TransportRole>,
        streams: Vec<StreamKind>,
    },
    Feature {
        version: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    Tcp,
    Websocket,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApplyPhase {
    Receiving,
    Validating,
    Saved,
    Applying,
    Applied,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationProgress {
    pub target: ConfigurationRef,
    pub phase: ApplyPhase,
    pub observed_at: DateTime<Utc>,
    pub problem: Option<StateProblem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationState {
    pub saved: Option<ConfigurationRef>,
    pub applied: Option<ConfigurationRef>,
    pub progress: Option<ConfigurationProgress>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ResourceRef {
    Listener { key: String },
    Dial { key: String },
    Coordination { key: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SocketNetwork {
    Udp,
    Tcp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum EndpointSource {
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointCandidate {
    pub address: String,
    pub origin: EndpointSource,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocketState {
    pub resource: ResourceRef,
    pub generation: Sequence,
    pub network: SocketNetwork,
    #[schema(value_type = String)]
    pub bound_address: SocketAddr,
    pub endpoints: Vec<EndpointCandidate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ListenerPhase {
    Preparing,
    Listening,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListenerState {
    pub phase: ListenerPhase,
    pub socket_ids: Vec<String>,
    pub loaded_certificate_sha256: Option<Digest>,
    pub problem: Option<StateProblem>,
    pub configuration: Option<ConfigurationRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkPhase {
    Preparing,
    Authenticated,
    Closed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkState {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReporterRole {
    Origin,
    Relay,
    Target,
    Exit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PathEvidence {
    Selected,
    Sent,
    PeerConfirmed,
    Delivered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathState {
    pub nodes: Vec<NodeId>,
    pub publication_id: PublicationId,
    pub reporter_role: ReporterRole,
    pub evidence: PathEvidence,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProbeKind {
    Neighbor,
    Path,
    Connect,
    Controller,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProbeOutcome {
    Pending,
    Success,
    Timeout,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeObservation {
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateProblem {
    ResourceUnavailable {
        resource: ResourceRef,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    ConfigurationRejected {
        target: ConfigurationRef,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    ProbeFailed {
        #[serde(rename = "measurementId")]
        measurement_id: EventId,
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionState {
    pub applied_profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeState {
    pub system: SystemState,
    pub capabilities: BTreeMap<String, Capability>,
    pub configuration: ConfigurationState,
    pub listeners: BTreeMap<String, ListenerState>,
    pub sockets: BTreeMap<String, SocketState>,
    pub links: BTreeMap<NodeId, Vec<LinkState>>,
    pub paths: BTreeMap<String, PathState>,
    pub selection: SelectionState,
    pub probes: Vec<ProbeObservation>,
    pub problems: Vec<StateProblem>,
}
