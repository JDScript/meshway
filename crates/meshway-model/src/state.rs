//! Source facts. These values neither grant authority nor contain transport secrets.
pub use crate::ConfigurationRef;
use crate::{Digest, EventId, NodeId, ObservationId, PublicationId, Sequence};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{collections::BTreeMap, net::SocketAddr};
use utoipa::ToSchema;

meshway_serde::object! {
/// Operating system and build information reported by the Node.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct SystemState {
    /// Hostname reported by the operating system, independent of the configured hostname.
    pub hostname: String,
    /// Operating system running the Node.
    pub os: String,
    /// Processor architecture of the running Node.
    pub arch: String,
    /// Version of the running Node software.
    pub agent_version: String,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Direction of transport establishment supported by the Node build.
pub enum TransportRole {
    /// The Node can initiate transport connections.
    Dial,
    /// The Node can accept transport connections.
    Listen,
}
}

meshway_serde::tagged! {
tag = "kind";
#[derive(Clone, Debug, PartialEq, Eq)]
/// Implementation capability declared by the Node, without granting execution permission.
pub enum Capability {
    /// Supported connection directions and stream encapsulations for a transport.
    Transport {
        /// Connection directions actually supported by this build.
        roles: Vec<TransportRole>,
        /// Stream encapsulations actually supported by this build.
        streams: Vec<StreamKind>,
    },
    /// Support for a versioned feature identified by the capability map key.
    Feature {
        /// Protocol version implemented for this feature.
        version: u16,
    },
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Outer stream encapsulation supported by a transport implementation.
pub enum StreamKind {
    /// TCP stream encapsulation is supported.
    Tcp,
    /// WebSocket stream encapsulation is supported.
    Websocket,
    /// UDP datagram encapsulation is supported.
    Udp,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Node-reported stage of receiving and installing an execution target.
pub enum ApplyPhase {
    /// The Node is receiving the target configuration.
    Receiving,
    /// The Node is validating the complete target configuration.
    Validating,
    /// The validated target has been privately persisted.
    Saved,
    /// The Node is installing resources and switching its local generation.
    Applying,
    /// All resources required by the Node have completed the local switch.
    Applied,
    /// A configuration processing or installation step failed.
    Failed,
}
}

meshway_serde::object! {
/// Node-reported progress for one exact execution target.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ConfigurationProgress {
    /// Exact configuration and execution publication being processed.
    pub target: ConfigurationRef,
    /// Actual processing or installation stage observed by the Node.
    pub phase: ApplyPhase,
    /// UTC time at which the Node observed this stage.
    pub observed_at: DateTime<Utc>,
    /// Reported failure details, or null when no problem is reported.
    pub problem: Option<StateProblem>,
} }

meshway_serde::object! {
/// Node-reported persistence, execution and progress facts, which do not prove business connectivity.
#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct ConfigurationState {
    /// Complete validated configuration privately persisted by the Node, or null if none is reported.
    pub saved: Option<ConfigurationRef>,
    /// Configuration used by the current running generation, or null if none is applied.
    pub applied: Option<ConfigurationRef>,
    /// Current target progress, or null when no progress is reported.
    pub progress: Option<ConfigurationProgress>,
} }

meshway_serde::tagged! { tag="kind";
/// Exact configured or execution resource associated with a runtime socket.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] pub enum ResourceRef {
    /// A configured inbound listener resource.
    Listener {
        /// Listener key in the associated configuration.
        key: String
    },
    /// An outbound dialing resource.
    Dial {
        /// Dial resource key in the associated execution configuration.
        key: String
    },
    /// A coordinated connection acquisition resource.
    Coordination {
        /// Coordination resource key in the associated execution configuration.
        key: String
    },
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Actual outer socket protocol, independent of the inner business protocol.
pub enum SocketNetwork {
    /// The socket carries UDP datagrams.
    Udp,
    /// The socket carries a TCP stream.
    Tcp,
}
}

meshway_serde::tagged! { tag="source";
/// Origin of an endpoint candidate, without asserting peer reachability.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum EndpointSource {
    /// The endpoint was explicitly supplied in the configuration.
    Configured {
        /// Path to the matching endpoint declaration in the stored configuration.
        #[serde(rename = "configPath")]
        config_path: String,
    },
    /// The Node obtained a usable address from a local interface.
    Interface {
        /// Local interface from which the address was obtained.
        interface: String,
    },
    /// A STUN exchange observed the mapping of this UDP socket.
    Stun {
        /// STUN observer used for the mapping observation.
        observer: String,
    },
} }

meshway_serde::object! {
/// Node-reported endpoint candidate bound to its original source and validity interval.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct EndpointCandidate {
    /// Host and port; interface and STUN candidates require an IP literal with a port.
    pub address: String,
    /// Source evidence for this candidate, not proof of reachability.
    pub origin: EndpointSource,
    /// Original UTC observation time, or the time a configured association was checked.
    pub observed_at: DateTime<Utc>,
    /// UTC deadline after which this candidate observation is no longer current.
    pub expires_at: DateTime<Utc>,
} }

meshway_serde::object! {
/// Actual outer socket reported by the Node and indexed by its instance-local socket ID.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct SocketState {
    /// Exact resource owning this socket.
    pub resource: ResourceRef,
    /// None inherits the report's applied configuration; installation sockets
    /// must name their target even before the first configuration is applied.
    pub configuration: Option<ConfigurationRef>,
    /// Resource generation that distinguishes replacement sockets using the same address.
    pub generation: Sequence,
    /// Outer protocol used by the socket.
    pub network: SocketNetwork,
    /// Address and port actually bound by the operating system.
    pub bound_address: SocketAddr [value_type = String],
    /// Candidates associated with this socket, retaining their original observation deadlines.
    pub endpoints: Vec<EndpointCandidate>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Local listener lifecycle reported by the Node.
pub enum ListenerPhase {
    /// The Node is preparing the listener resources.
    Preparing,
    /// Local listening succeeded, without proving public reachability.
    Listening,
    /// The listener has stopped.
    Stopped,
    /// Listener preparation or operation failed.
    Failed,
}
}

meshway_serde::object! {
/// Runtime facts for a configured listener key.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ListenerState {
    /// Local lifecycle stage observed by the Node.
    pub phase: ListenerPhase,
    /// Actual socket IDs in the same Node report.
    pub socket_ids: Vec<String>,
    /// Public fingerprint of the certificate actually loaded, or null for non-TLS or unavailable results.
    pub loaded_certificate_sha256: Option<Digest>,
    /// Typed listener failure, or null when none is reported.
    pub problem: Option<StateProblem>,
    /// Explicit execution target; null or omission inherits the report's applied configuration.
    pub configuration: Option<ConfigurationRef>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Node-reported lifecycle of an actual transport channel.
pub enum LinkPhase {
    /// Channel establishment or authentication is in progress.
    Preparing,
    /// Channel authentication completed and the channel still exists, possibly idle.
    Authenticated,
    /// The Node observed that the channel closed.
    Closed,
    /// Channel establishment or operation failed.
    Failed,
}
}

meshway_serde::object! {
/// One endpoint's report of a physical channel and its current dependencies.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct LinkState {
    /// Non-secret channel identifier bound by the actual handshake.
    pub link_id: String,
    /// Transport actually used by this channel.
    pub transport: String,
    /// Node that initiated establishment, independent of business traffic direction.
    pub initiator_node_id: NodeId,
    /// Socket carrying this channel in the same report.
    pub socket_id: String,
    /// Channel lifecycle stage observed by this endpoint.
    pub phase: LinkPhase,
    /// Number of current business dependencies, not a traffic counter or rate.
    pub business_dependencies: u32,
    /// Diagnostic tasks depending on this channel, separate from business dependencies.
    pub observation_ids: Vec<ObservationId>,
    /// Original UTC time at which the Node observed this channel fact.
    pub observed_at: DateTime<Utc>,
    /// UTC deadline after which this channel report is no longer current.
    pub expires_at: DateTime<Utc>,
    /// Known UTC start of the reported state, or null when unknown.
    pub since: Option<DateTime<Utc>>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Reporting Node's role in the complete path it observed.
pub enum ReporterRole {
    /// The reporter originates the path's business traffic.
    Origin,
    /// The reporter forwards traffic as an intermediate Node.
    Relay,
    /// The reporter is the path's business destination.
    Target,
    /// The reporter occupies the path's exit role.
    Exit,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Stage of path use actually observable by the reporting Node.
pub enum PathEvidence {
    /// The reporter selected the path without claiming traffic delivery.
    Selected,
    /// The reporter sent traffic on the path.
    Sent,
    /// The reporter obtained peer confirmation for the path.
    PeerConfirmed,
    /// The reporter observed delivery at the destination.
    Delivered,
}
}

meshway_serde::object! {
/// A participating Node's observation of a complete runtime path.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PathState {
    /// Actual complete ordered Node path.
    pub nodes: Vec<NodeId>,
    /// Execution publication under which this path was used.
    pub publication_id: PublicationId,
    /// Reporting Node's role in the path.
    pub reporter_role: ReporterRole,
    /// Stage of path use observed by this reporter.
    pub evidence: PathEvidence,
    /// Original UTC observation time.
    pub observed_at: DateTime<Utc>,
    /// UTC deadline after which this path observation is no longer current.
    pub expires_at: DateTime<Utc>,
} }

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Operation measured by a probe; different kinds have different timing boundaries.
pub enum ProbeKind {
    /// Probe over an already authenticated adjacent channel.
    Neighbor,
    /// Endpoint probe over a specified complete path.
    Path,
    /// Transport discovery, establishment and handshake duration.
    Connect,
    /// Control-plane request duration.
    Controller,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Actual outcome of a measured operation, without replacing missing timing with zero.
pub enum ProbeOutcome {
    /// The operation has not reached a final outcome.
    Pending,
    /// The measured operation completed successfully.
    Success,
    /// The operation did not complete within its deadline.
    Timeout,
    /// The operation failed for a reason other than its timeout outcome.
    Failed,
}
}

meshway_serde::object! {
/// Node-reported measurement with original timing and execution context.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeObservation {
    /// Stable measurement identifier reused when the same observation is reported again.
    pub measurement_id: EventId,
    /// Measured operation and its timing scope.
    pub kind: ProbeKind,
    /// Measured channel identifier, or null when no channel is associated.
    pub link_id: Option<String>,
    /// Measured transport, or null when no transport is associated.
    pub transport: Option<String>,
    /// Actual physical request/reply channels, when measured by the executor.
    pub channels: Option<crate::observations::ProbeChannels>,
    /// Ordered Node path associated with the measured operation.
    pub path: Vec<NodeId>,
    /// Execution publication associated with the measurement.
    pub publication_id: PublicationId,
    /// UTC start time used to place the measurement in history.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while no completion is recorded.
    pub completed_at: Option<DateTime<Utc>>,
    /// Actual outcome of the measured operation.
    pub outcome: ProbeOutcome,
    /// Monotonic elapsed microseconds, or null when no real timing is available.
    pub duration_us: Option<Sequence>,
} }

meshway_serde::tagged! {
tag = "code";
#[derive(Clone, Debug, PartialEq, Eq)]
/// Typed Node-reported failure retaining the affected resource or measurement.
pub enum StateProblem {
    /// A required runtime resource is unavailable.
    ResourceUnavailable = "resource_unavailable" {
        /// Resource that the Node could not use.
        resource: ResourceRef,
        /// UTC time at which the Node observed the failure.
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    /// The Node rejected an execution target.
    ConfigurationRejected = "configuration_rejected" {
        /// Exact target rejected by the Node.
        target: ConfigurationRef,
        /// UTC time at which the Node observed the rejection.
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
    /// A specific measurement failed.
    ProbeFailed = "probe_failed" {
        /// Identifier of the failed measurement.
        #[serde(rename = "measurementId")]
        measurement_id: EventId,
        /// UTC time at which the Node observed the failure.
        #[serde(rename = "observedAt")]
        observed_at: DateTime<Utc>,
    },
}
}

meshway_serde::object! {
/// Node-reported routing profile currently in use.
#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct SelectionState {
    /// Actually applied profile within configuration constraints, or null when none is reported.
    pub applied_profile: Option<String>,
} }

meshway_serde::object! {
/// Complete snapshot of a Node's own runtime facts, without granting authority or extending leases.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct NodeState {
    /// Operating system and build facts reported by the Node.
    pub system: SystemState,
    /// Namespaced build capabilities matching the authenticated instance acquisition.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub capabilities: BTreeMap<String, Capability>,
    /// Independently reported saved target, applied target and installation progress.
    pub configuration: ConfigurationState,
    /// Complete listener inventory indexed by configured listener key.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub listeners: BTreeMap<String, ListenerState>,
    /// Complete outer socket inventory indexed by instance-local socket ID.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub sockets: BTreeMap<String, SocketState>,
    /// Actual channels grouped by peer, including simultaneous channels to the same peer.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub links: BTreeMap<NodeId, Vec<LinkState>>,
    /// Paths observed by this Node, indexed by runtime path identifier.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub paths: BTreeMap<String, PathState>,
    /// Routing selection actually applied by the Node.
    pub selection: SelectionState,
    /// Recent real measurements; repeating a snapshot does not create new samples.
    pub probes: Vec<ProbeObservation>,
    /// Current typed failures observed by the Node.
    pub problems: Vec<StateProblem>,
} }
