use crate::{
    Digest, EventId, NodeId, Sequence, SocketAddress, UdpTraversalId,
    state::{ConfigurationRef, ProbeObservation},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

meshway_serde::string_enum! {
/// Coarse provenance only; private observer and interface details are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EndpointKind {
    /// The endpoint was explicitly declared in configuration.
    Configured,
    /// The endpoint came from a local interface observation.
    Interface,
    /// The endpoint came from a STUN mapping observation.
    Stun,
}
}

meshway_serde::tagged! {
/// How the actual physical channel was acquired, not business authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAcquisition {
    /// The channel was acquired by dialing a published endpoint.
    PublishedEndpoint,
    /// The channel was acquired by accepting an inbound connection.
    AcceptedInbound,
    /// The UDP channel was acquired through a coordinated traversal attempt.
    CoordinatedUdp {
        /// Coordinated UDP traversal attempt that acquired the channel.
        #[serde(rename = "attemptId")]
        attempt_id: UdpTraversalId,
        /// Coarse provenance of the endpoint used for the traversal.
        #[serde(rename = "endpointSource")]
        endpoint_source: EndpointKind,
    },
}
}

meshway_serde::object! {
/// Physical channel actually used for one side of a measured probe exchange.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeChannel {
    /// Non-secret authenticated channel identifier observed by the executor.
    pub link_id: Digest,
    /// Actual local socket identifier carrying the probe.
    pub socket_id: Digest,
    /// Adjacent Node authenticated on this physical channel.
    pub peer_node_id: NodeId,
    /// Actual outer transport used by the channel.
    pub transport: meshway_transport_spec::TransportProtocol,
    /// Local socket address used by this channel.
    pub local_address: SocketAddress,
    /// Remote socket address used by this channel.
    pub remote_address: SocketAddress,
    /// How the executor acquired this physical channel.
    pub acquisition: LinkAcquisition,
} }

meshway_serde::object! {
/// Actual request and reply channel evidence supplied by the probe executor.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeChannels {
    /// Physical channel used to send the measured request.
    pub request: ProbeChannel,
    /// Null until a reply has actually been observed on a channel.
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub reply: Option<ProbeChannel> [required = true],
} }

meshway_serde::object! {
/// Real Node event retained with its original execution context and occurrence time.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ObservationEvent {
    /// Stable event identifier used to deduplicate reports from this source Node.
    pub event_id: EventId,
    /// Exact execution target under which the event occurred.
    pub configuration: ConfigurationRef,
    /// Original UTC occurrence time, unchanged by delivery retries.
    pub occurred_at: DateTime<Utc>,
    /// Typed event facts reported by the Node's connection or measurement owner.
    pub payload: ObservationPayload,
} }

meshway_serde::tagged! {
tag = "kind";
#[derive(Clone, Debug, PartialEq, Eq)]
/// Node-reported connection, probe or collection-gap event facts.
pub enum ObservationPayload {
    /// A real connection establishment attempt began.
    DialStarted = "dial_started" {
        /// Intended peer of the connection attempt.
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        /// Identifier correlating events for the same establishment attempt.
        #[serde(rename = "attemptId")]
        attempt_id: String,
        /// Transport attempted by the Node.
        transport: String,
    },
    /// A real connection establishment attempt failed.
    DialFailed = "dial_failed" {
        /// Intended peer of the failed connection attempt.
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        /// Identifier of the failed establishment attempt.
        #[serde(rename = "attemptId")]
        attempt_id: String,
        /// Transport attempted by the Node.
        transport: String,
        /// Typed reason the establishment attempt ended.
        reason: CloseReason,
        /// Monotonic attempt duration in microseconds, or null when no timing is available.
        #[serde(rename = "durationUs")]
        duration_us: Option<Sequence>,
    },
    /// Authentication completed for an actual channel.
    LinkOpened = "link_opened" {
        /// Adjacent Node authenticated on the channel.
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        /// Actual channel identifier bound by the handshake.
        #[serde(rename = "linkId")]
        link_id: String,
        /// Transport used by the authenticated channel.
        transport: String,
    },
    /// The Node observed an actual channel close.
    LinkClosed = "link_closed" {
        /// Adjacent Node of the closed channel.
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        /// Identifier of the channel that closed.
        #[serde(rename = "linkId")]
        link_id: String,
        /// Transport used by the closed channel.
        transport: String,
        /// Typed reason for the observed closure.
        reason: CloseReason,
    },
    /// A real measured operation reached its reported outcome.
    ProbeCompleted = "probe_completed" {
        /// Measurement with its original identity, context and timing boundaries.
        probe: ProbeObservation,
    },
    /// The Node's event collector lost observations within a known interval.
    CollectionGap = "collection_gap" {
        /// Number of events actually dropped, not a count of lost network packets.
        dropped: Sequence,
        /// Original UTC start of the collection gap.
        from: DateTime<Utc>,
        /// Original UTC end of the collection gap.
        to: DateTime<Utc>,
    },
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Node-reported reason a channel or connection attempt ended.
pub enum CloseReason {
    /// The connection owner reclaimed an idle channel.
    Idle,
    /// The remote endpoint closed the channel.
    RemoteClosed,
    /// The operation or channel reached its applicable timeout.
    Timeout,
    /// The execution authorization or resource was withdrawn.
    Withdrawn,
    /// A local operation or resource failure ended the attempt or channel.
    LocalFailure,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Requested observation metric, whose availability is reported separately.
pub enum MetricKind {
    /// Measured probe round-trip duration using actual timing boundaries.
    Rtt,
    /// Source-scoped business traffic counters; currently reported as unsupported.
    Traffic,
    /// Fraction of sent probes without a reply before their deadline, not wire packet loss.
    ProbeLoss,
}
}

meshway_serde::object! {
/// Source-scoped business byte counters for a real measurement window; traffic is currently unsupported.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct TrafficWindow {
    /// Mesh IP business bytes accepted at the source and submitted to the path, excluding control and probe traffic.
    pub tx_bytes: Sequence,
    /// Mesh IP business bytes verified on the return path and delivered locally at the source.
    pub rx_bytes: Sequence,
    /// Counter generation that changes on reset so old delta baselines cannot be reused.
    pub counter_generation: Sequence,
} }
meshway_serde::object! {
/// Counts of actually sent probes and their deadline outcomes within a measurement window.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeWindow {
    /// Number of probes actually sent, excluding preparation and unsent local failures.
    pub sent: u32,
    /// Sent probes whose replies arrived within their deadline.
    pub replied_within_deadline: u32,
    /// Sent probes that reached their deadline without an on-time reply.
    pub timeout: u32,
    /// Sent probes still awaiting a final deadline outcome, excluded from the unanswered-rate denominator.
    pub pending: u32,
} }
meshway_serde::object! {
/// Actual measurement window; revisions replace the same window rather than adding synthetic samples.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ObservationSample {
    /// Stable measurement-window identifier used for deduplication and replacement.
    pub measurement_id: EventId,
    /// Revisions replace a partial window rather than creating another sample.
    pub version: crate::Version,
    /// UTC start of the original measurement window.
    pub window_start: DateTime<Utc>,
    /// UTC end of the original measurement window, after its start.
    pub window_end: DateTime<Utc>,
    /// Whether the window still has incomplete outcomes and may be revised.
    pub partial: bool,
    /// Actual probe measurement, or null when this sample has no probe result.
    pub probe: Option<ProbeObservation>,
    /// Business traffic counters, currently null because traffic measurement is unsupported.
    pub traffic: Option<TrafficWindow>,
    /// Sent-probe deadline counts, or null when this metric is unavailable for the sample.
    pub probe_counts: Option<ProbeWindow>,
} }
