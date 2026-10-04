use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{
    InstanceId, NodeId, ObservationId, ObservationSubscriptionId, Sequence,
    observations::{MetricKind, ObservationEvent, ObservationSample},
    state::ConfigurationRef,
};

wire_object! { #[derive(Clone, Debug)] pub struct HistoryReport { pub instance_id: InstanceId, pub events: Vec<ObservationEvent> } }
wire_object! { #[derive(Clone, Debug)] pub struct HistoryAccepted { pub accepted: u32, pub duplicates: u32 } }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum HistoryMode {
    Raw,
    Buckets,
    RecentLinks,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    DialStarted,
    DialFailed,
    LinkOpened,
    LinkClosed,
    ProbeCompleted,
    CollectionGap,
}
wire_object! { #[derive(Clone, Debug)] pub struct HistoryQuery {
    pub from: DateTime<Utc>, pub to: DateTime<Utc>, pub source_node_id: Option<NodeId>,
    pub peer_node_id: Option<NodeId>, pub mode: HistoryMode, pub bucket_seconds: Option<u32>,
    pub cursor: Option<String>, pub limit: u16 [minimum=1,maximum=200],
    pub kinds: Option<Vec<HistoryKind>>, pub link_id: Option<String>,
} }
wire_object! { #[derive(Clone, Debug)] pub struct HistoricalEvent {
    pub source_node_id: NodeId, pub instance_id: InstanceId, pub received_at: DateTime<Utc>, pub event: ObservationEvent,
} }
wire_object! { #[derive(Clone, Debug)] pub struct HistoryBucket {
    pub from: DateTime<Utc>, pub to: DateTime<Utc>, pub source_node_id: NodeId,
    pub sample_count: u32, pub success: u32, pub timeout: u32, pub failed: u32,
    pub mean_success_duration_us: Option<f64>,
    pub instance_id: InstanceId, pub publication_id: meshway_model::PublicationId,
    pub path: Vec<NodeId>, pub kind: meshway_model::state::ProbeKind,
    pub link_id: Option<String>, pub transport: Option<String>,
} }
wire_object! { #[derive(Clone, Debug)] pub struct RecentLink {
    pub source_node_id: NodeId, pub peer_node_id: NodeId, pub link_id: String,
    pub transport: String, pub last_event_at: DateTime<Utc>, pub closed: bool,
} }
crate::object::wire_tagged! { #[derive(Clone, Debug)] pub enum HistoryCoverage {
    Pruned {
        #[serde(rename = "sourceNodeId")]
        source_node_id: NodeId,
        before: DateTime<Utc>,
        reason: CoverageReason,
    },
    CollectionGap {
        #[serde(rename = "sourceNodeId")]
        source_node_id: NodeId,
        #[serde(rename = "instanceId")]
        instance_id: InstanceId,
        #[serde(rename = "eventId")]
        event_id: meshway_model::EventId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        dropped: Sequence,
    },
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum CoverageReason {
    Retention,
    Capacity,
}
crate::object::wire_tagged! { tag="mode"; #[derive(Clone, Debug)] pub enum HistoryItems {
    Raw { items: Vec<HistoricalEvent> },
    Buckets { items: Vec<HistoryBucket> },
    RecentLinks { items: Vec<RecentLink> },
} }
wire_object! { #[derive(Clone, Debug)] pub struct HistoryPage {
    pub from: DateTime<Utc>, pub to: DateTime<Utc>, pub as_of: DateTime<Utc>, pub next_cursor: Option<String>,
    pub coverage: Vec<HistoryCoverage>, pub result: HistoryItems,
} }
crate::object::wire_tagged! { tag="mode"; #[derive(Clone, Debug, PartialEq, Eq)] pub enum ObservationPath {
    Auto,
    Explicit { path: Vec<NodeId> },
} }
wire_object! { #[derive(Clone, Debug)] pub struct CreateObservation {
    pub subscription_id: ObservationSubscriptionId,
    pub source_node_id: NodeId, pub target_node_id: NodeId, pub selection: ObservationPath,
    pub service: meshway_configuration::execution::ObservationServiceScope,
    pub metrics: Vec<MetricKind>, pub interval_ms: u32 [minimum=1000,maximum=30000],
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ObservationPhase {
    Preparing,
    Running,
    Completed,
    Failed,
    Expired,
    Cancelled,
    Unsupported,
}
wire_object! { #[derive(Clone, Debug)] pub struct ObservationTask {
    pub id: ObservationId, pub source_node_id: NodeId, pub target_node_id: NodeId,
    pub service: meshway_configuration::execution::ObservationServiceScope,
    pub selection: ObservationPath, pub path: Option<Vec<NodeId>>, pub path_segment: Option<Sequence>,
    pub configuration: ConfigurationRef, pub instance_id: InstanceId,
    pub metrics: Vec<MetricKind>, pub interval_ms: u32, pub phase: ObservationPhase,
    pub unsupported_metrics: Vec<MetricKind>,
    pub expires_at: DateTime<Utc>, pub renew_after: DateTime<Utc>, pub latest: Option<ObservationSample>,
    pub subscriber_expires_at: Option<DateTime<Utc>>,
    pub subscription_id: Option<ObservationSubscriptionId>,
} }
wire_object! { #[derive(Clone, Debug)] pub struct ObservationList { pub items: Vec<ObservationTask> } }
wire_object! { #[derive(Clone, Debug)] pub struct ObservationResult {
    pub instance_id: InstanceId, pub configuration: ConfigurationRef, pub result: ObservationUpdate,
} }
crate::object::wire_tagged! { #[derive(Clone, Debug)] pub enum ObservationUpdate {
    Phase {
        phase: ReportedObservationPhase,
    },
    Sample {
        path: Vec<NodeId>,
        #[serde(rename = "pathSegment")]
        path_segment: Sequence,
        sample: Box<ObservationSample>,
    },
} }
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReportedObservationPhase {
    Preparing,
    Failed,
    Completed,
    Unsupported,
}
