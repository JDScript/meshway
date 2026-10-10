use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{
    InstanceId, NetmapRef, NodeId, ObservationId, ObservationSubscriptionId, Sequence,
    observations::{MetricKind, ObservationEvent, ObservationSample},
};

wire_object! {
/// Batch of real historical events reported by an authenticated Node.
#[derive(Clone, Debug)] pub struct HistoryReport {
    /// Runtime instance that produced the events.
    pub instance_id: InstanceId,
    /// Events retaining their original identifiers, occurrence times and execution contexts.
    pub events: Vec<ObservationEvent>
} }
wire_object! {
/// Server acceptance counts for an idempotent historical event batch.
#[derive(Clone, Debug)] pub struct HistoryAccepted {
    /// Number of newly accepted events in the batch.
    pub accepted: u32,
    /// Number of events already accepted with the same source, identifier and content.
    pub duplicates: u32
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Projection requested from stored observation history, without initiating new measurements.
pub enum HistoryMode {
    /// Return original historical events.
    Raw,
    /// Return time buckets aggregating real probe results.
    Buckets,
    /// Return recent channel summaries derived from actual connection events.
    RecentLinks,
}
}
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
/// Historical event kind used to filter a query.
pub enum HistoryKind {
    /// A connection establishment attempt began.
    DialStarted,
    /// A connection establishment attempt failed.
    DialFailed,
    /// An actual channel completed authentication.
    LinkOpened,
    /// An actual channel closed.
    LinkClosed,
    /// A measured operation completed.
    ProbeCompleted,
    /// The source collector reported dropped events over an interval.
    CollectionGap,
}
}
wire_object! {
/// Fixed-window history query evaluated under the caller's current permissions.
#[derive(Clone, Debug)] pub struct HistoryQuery {
    /// UTC start of the requested event-time window.
    pub from: DateTime<Utc>,
    /// UTC end of the requested event-time window.
    pub to: DateTime<Utc>,
    /// Optional reporting Node filter that only narrows authorized results.
    pub source_node_id: Option<NodeId>,
    /// Optional adjacent peer filter for relevant events.
    pub peer_node_id: Option<NodeId>,
    /// Raw events, probe buckets or recent channel summaries to return.
    pub mode: HistoryMode,
    /// Required bucket width of 1–86400 seconds in bucket mode; null or omitted in other modes.
    pub bucket_seconds: Option<u32>,
    /// Continuation cursor bound to this window, filters, mode, identity and read upper bound.
    pub cursor: Option<String>,
    /// Maximum number of result items in the page, from 1 through 200.
    pub limit: u16 [minimum=1,maximum=200],
    /// Optional event-kind filter; coverage reporting remains independent of this filter.
    pub kinds: Option<Vec<HistoryKind>>,
    /// Optional actual channel identifier filter.
    pub link_id: Option<String>,
} }
wire_object! {
/// Stored event with authenticated source identity and original Server reception time.
#[derive(Clone, Debug)] pub struct HistoricalEvent {
    /// Authenticated Node that reported the event.
    pub source_node_id: NodeId,
    /// Runtime instance under which the event was reported.
    pub instance_id: InstanceId,
    /// UTC time at which the Server first accepted the event.
    pub received_at: DateTime<Utc>,
    /// Original event facts, occurrence time and execution target.
    pub event: ObservationEvent,
} }
wire_object! {
/// Aggregate of real probe results sharing a source, execution context and measurement scope.
#[derive(Clone, Debug)] pub struct HistoryBucket {
    /// UTC start of the aggregation bucket.
    pub from: DateTime<Utc>,
    /// UTC end of the aggregation bucket.
    pub to: DateTime<Utc>,
    /// Node that produced the aggregated measurements.
    pub source_node_id: NodeId,
    /// Number of real probe results contributing to the bucket.
    pub sample_count: u32,
    /// Number of successful measured operations.
    pub success: u32,
    /// Number of measured operations that timed out.
    pub timeout: u32,
    /// Number of measured operations that failed.
    pub failed: u32,
    /// Mean available duration of successful operations in microseconds, or null without timed successes.
    pub mean_success_duration_us: Option<f64>,
    /// Source instance shared by the aggregated measurements.
    pub instance_id: InstanceId,
    /// Netmap shared by the aggregated measurements.
    pub netmap: NetmapRef,
    /// Ordered path shared by the aggregated measurements.
    pub path: Vec<NodeId>,
    /// Measured operation kind, preserving its timing scope.
    pub kind: meshway_model::state::ProbeKind,
    /// Shared channel identifier, or null when the measurements have none.
    pub link_id: Option<String>,
    /// Shared transport, or null when the measurements have none.
    pub transport: Option<String>,
} }
wire_object! {
/// Recent channel summary derived from stored events, not a claim of current connectivity.
#[derive(Clone, Debug)] pub struct RecentLink {
    /// Node that reported the channel events.
    pub source_node_id: NodeId,
    /// Adjacent peer identified by the channel events.
    pub peer_node_id: NodeId,
    /// Actual channel identifier retained from the events.
    pub link_id: String,
    /// Transport used by the reported channel.
    pub transport: String,
    /// UTC occurrence time of the latest contributing event.
    pub last_event_at: DateTime<Utc>,
    /// Whether the latest contributing channel event reports closure.
    pub closed: bool,
} }
crate::object::wire_tagged! {
/// Known retention boundaries and source collection gaps affecting historical coverage.
#[derive(Clone, Debug)] pub enum HistoryCoverage {
    /// The Server pruned retained history for this source before a known boundary.
    Pruned {
        /// Source Node whose retained history was pruned.
        #[serde(rename = "sourceNodeId")]
        source_node_id: NodeId,
        /// UTC boundary before which the stated pruning affects coverage.
        before: DateTime<Utc>,
        /// Retention or capacity policy responsible for pruning.
        reason: CoverageReason,
    },
    /// A source-reported collection gap intersects the query window.
    CollectionGap {
        /// Node whose collector dropped the events.
        #[serde(rename = "sourceNodeId")]
        source_node_id: NodeId,
        /// Runtime instance that reported the collection gap.
        #[serde(rename = "instanceId")]
        instance_id: InstanceId,
        /// Original collection-gap event identifier.
        #[serde(rename = "eventId")]
        event_id: meshway_model::EventId,
        /// Original UTC start of the gap, without clipping to the query window.
        from: DateTime<Utc>,
        /// Original UTC end of the gap, without clipping to the query window.
        to: DateTime<Utc>,
        /// Original number of dropped events, without estimating a query-window proportion.
        dropped: Sequence,
    },
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Server policy responsible for pruning historical coverage.
pub enum CoverageReason {
    /// Events exceeded the configured retention lifetime.
    Retention,
    /// Events were removed to satisfy bounded storage capacity.
    Capacity,
}
}
crate::object::wire_tagged! { tag="mode";
/// Historical result projection matching the requested query mode.
#[derive(Clone, Debug)] pub enum HistoryItems {
    /// Original events with authenticated source and reception metadata.
    Raw {
        /// Visible events within the bounded query page.
        items: Vec<HistoricalEvent>
    },
    /// Aggregated real probe results without synthetic empty buckets.
    Buckets {
        /// Nonempty measurement buckets within the query page.
        items: Vec<HistoryBucket>
    },
    /// Channel summaries derived from actual historical events.
    RecentLinks {
        /// Visible recent channels within the query page.
        items: Vec<RecentLink>
    },
} }
wire_object! {
/// Permission-filtered historical page with explicit coverage and a stable continuation boundary.
#[derive(Clone, Debug)] pub struct HistoryPage {
    /// UTC start of the requested history window.
    pub from: DateTime<Utc>,
    /// UTC end of the requested history window.
    pub to: DateTime<Utc>,
    /// UTC read upper bound retained across continuation pages.
    pub as_of: DateTime<Utc>,
    /// Cursor for continuing this exact query, or null when no page remains.
    pub next_cursor: Option<String>,
    /// Known pruning and collection gaps visible under current source permissions.
    pub coverage: Vec<HistoryCoverage>,
    /// Historical items in the requested projection mode.
    pub result: HistoryItems,
} }
crate::object::wire_tagged! { tag="mode";
/// Path selection constraint for an explicitly authorized diagnostic observation.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum ObservationPath {
    /// Select an executable complete path from the authorized candidates.
    Auto,
    /// Use only the specified complete path without bypassing its required Nodes.
    Explicit {
        /// Fixed ordered path including the source, required intermediate Nodes and target.
        path: Vec<NodeId>
    },
} }
wire_object! {
/// User request for a short-lived, independently authorized diagnostic subscription.
#[derive(Clone, Debug)] pub struct CreateObservation {
    /// Independent subscription identifier reused for bounded idempotent creation retries.
    pub subscription_id: ObservationSubscriptionId,
    /// Node authorized to originate the measurement; ordinary members may use only their own User Nodes.
    pub source_node_id: NodeId,
    /// Business destination Node within the current network.
    pub target_node_id: NodeId,
    /// Automatic authorized selection or an explicit complete path.
    pub selection: ObservationPath,
    /// Exact TCP, UDP or ICMP service scope checked by access and routing policy.
    pub service: meshway_configuration::execution::ObservationServiceScope,
    /// Requested measurement kinds; unavailable kinds are returned separately as unsupported.
    pub metrics: Vec<MetricKind>,
    /// Requested sampling interval in milliseconds, from 1000 through 30000 and subject to budgets.
    pub interval_ms: u32 [minimum=1000,maximum=30000],
} }
meshway_model::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Current task or subscription lifecycle phase, distinct from individual probe outcomes.
pub enum ObservationPhase {
    /// The source is preparing execution or acquiring an authorized path.
    Preparing,
    /// The source has reported a sample for an actual selected path.
    Running,
    /// The source reported normal completion.
    Completed,
    /// The source reported that the observation failed.
    Failed,
    /// The Server determined that the observation or subscription lease expired.
    Expired,
    /// The observation subscription was cancelled.
    Cancelled,
    /// The requested observation cannot be provided by the available execution capabilities.
    Unsupported,
}
}
wire_object! {
/// Shared diagnostic task with an optional independently leased subscriber projection.
#[derive(Clone, Debug)] pub struct ObservationTask {
    /// Shared Node measurement task identifier, distinct from a subscriber identifier.
    pub id: ObservationId,
    /// Node authorized to execute and report the measurement.
    pub source_node_id: NodeId,
    /// Authorized business destination of the measured path.
    pub target_node_id: NodeId,
    /// Exact service scope covered by the diagnostic permission.
    pub service: meshway_configuration::execution::ObservationServiceScope,
    /// Authorized path-selection mode and any fixed complete path.
    pub selection: ObservationPath,
    /// Actual complete measured path, or null until a path is selected and reported.
    pub path: Option<Vec<NodeId>>,
    /// Actual path segment identifier, or null before a measured segment is reported.
    pub path_segment: Option<Sequence>,
    /// Source netmap current when the task was assigned; results name the netmap they ran under.
    pub netmap: NetmapRef,
    /// Source runtime instance authorized to execute this task.
    pub instance_id: InstanceId,
    /// Measurement kinds assigned to this task.
    pub metrics: Vec<MetricKind>,
    /// Actual sampling interval in milliseconds after budget constraints.
    pub interval_ms: u32,
    /// Current task or subscriber lifecycle phase.
    pub phase: ObservationPhase,
    /// Requested metrics the Server cannot provide; traffic is currently unsupported.
    pub unsupported_metrics: Vec<MetricKind>,
    /// UTC task deadline beyond which execution and results are no longer authorized.
    pub expires_at: DateTime<Utc>,
    /// UTC time after which the subscriber should renew under current permissions.
    pub renew_after: DateTime<Utc>,
    /// Latest accepted real measurement window, or null before a valid sample is available.
    pub latest: Option<ObservationSample>,
    /// Independent subscriber's UTC lease deadline, or null in a Node task projection.
    pub subscriber_expires_at: Option<DateTime<Utc>>,
    /// Subscriber identifier for this projection, or null when returning the shared Node task.
    pub subscription_id: Option<ObservationSubscriptionId>,
} }
wire_object! {
/// Observation tasks or subscriber projections visible to the authenticated caller.
#[derive(Clone, Debug)] pub struct ObservationList {
    /// Currently visible observation results and lifecycle states.
    pub items: Vec<ObservationTask>
} }
wire_object! {
/// Source Node result bound to the task's actual instance and netmap.
#[derive(Clone, Debug)] pub struct ObservationResult {
    /// Source instance that produced the update.
    pub instance_id: InstanceId,
    /// Exact netmap under which the update was produced.
    pub netmap: NetmapRef,
    /// Actual phase or sample reported by the source Node.
    pub result: ObservationUpdate,
} }
crate::object::wire_tagged! {
/// Source Node update separating lifecycle progress from real measured windows.
#[derive(Clone, Debug)] pub enum ObservationUpdate {
    /// Report lifecycle progress without inventing a path or empty sample.
    Phase {
        /// Lifecycle phase actually observed by the source executor.
        phase: ReportedObservationPhase,
    },
    /// Report a nonempty measurement window for an actual path segment.
    Sample {
        /// Actual complete ordered path used for this measurement.
        path: Vec<NodeId>,
        /// Segment identifier separating results across actual path selections.
        #[serde(rename = "pathSegment")]
        path_segment: Sequence,
        /// Actual measurement window with its stable identifier and revision.
        sample: Box<ObservationSample>,
    },
} }
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Lifecycle phases a Node may report without supplying a measurement sample.
pub enum ReportedObservationPhase {
    /// The executor is preparing execution or acquiring the authorized path.
    Preparing,
    /// The executor encountered a failure that ended the observation.
    Failed,
    /// The executor completed the observation.
    Completed,
    /// The executor cannot provide the assigned observation.
    Unsupported,
}
}
