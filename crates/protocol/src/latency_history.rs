//! Immutable, directed authenticated-neighbor probe events. Never E2E latency.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ConnectionTransport;

pub const LATENCY_SAMPLE_INTERVAL_SECONDS: u32 = 30;
pub const LATENCY_RETENTION_SECONDS: u32 = 7 * 86400;
pub const LATENCY_BATCH_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LatencyOutcome {
    Success,
    Timeout,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LatencyFailureReason {
    ChannelUnavailable,
    AuthenticationFailed,
    LocalSendFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LatencyDirection {
    SourceToPeer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyChannelAssociation {
    pub transport: ConnectionTransport,
    pub initiator_node_id: String,
    pub listener_id: String,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub candidate_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencySample {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub measurement_id: String,
    pub peer_node_id: String,
    pub direction: LatencyDirection,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub runtime_epoch: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    /// Local sampler generation, not an assertion of physical channel identity.
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub sampler_generation: String,
    /// Actual probe invocation start; retained unchanged through queue/retry.
    pub measured_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub outcome: LatencyOutcome,
    /// One sanitized attempt failure for Failed, otherwise empty. This is not
    /// a network loss classification; local send failures are kept separate.
    #[schema(max_items = 1)]
    pub failure_reasons: Vec<LatencyFailureReason>,
    /// Runtime returns exact channel metadata only with successful proof. A
    /// timeout leaves this empty rather than racing a separate registry read.
    /// Exactly one association for success, zero for timeout.
    #[schema(max_items = 1)]
    pub channel_associations: Vec<LatencyChannelAssociation>,
    /// Integer microseconds, measured using a monotonic clock, never HTTP RTT.
    #[schema(required = true, format = Int64, minimum = 1, maximum = 60000000)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rtt_us: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportLatencySamplesRequest {
    #[schema(min_items = 1, max_items = 64)]
    pub samples: Vec<LatencySample>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportLatencySamplesResponse {
    #[schema(format = Int64, minimum = 0, maximum = 64)]
    pub inserted: u32,
    #[schema(format = Int64, minimum = 0, maximum = 64)]
    pub duplicates: u32,
}

/// Raw events for one directed pair. Exclusive row cursor, not an offset.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryLatencyHistoryRequest {
    pub source_node_id: String,
    pub peer_node_id: String,
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    #[schema(format = Int64, minimum = 1, maximum = 1000)]
    pub limit: u32,
    #[schema(required = true, min_length = 1, max_length = 20, pattern = "^[0-9]+$")]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub after: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyHistoryRecord {
    pub cursor: String,
    pub network_id: String,
    pub source_node_id: String,
    pub sample: LatencySample,
    pub received_at: DateTime<Utc>,
    /// Always true: a retained measurement never asserts present connectivity.
    pub historical: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyHistoryResponse {
    pub records: Vec<LatencyHistoryRecord>,
    /// Current TTL floor. This is a maximum age, not a promise of continuous data.
    pub retention_from: DateTime<Utc>,
    /// Conservative source-wide capacity watermark (all peers). Some earlier
    /// events were evicted; it does not describe offline time or continuous coverage.
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub capacity_trimmed_before: Option<DateTime<Utc>>,
    /// The requested window intersects the conservative capacity-trimmed range.
    pub capacity_trimmed: bool,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub next: Option<String>,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub retention_seconds: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_interval_seconds: u32,
}
