//! Leased, node-scoped runtime self-reports, separate from saved configuration ACKs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ConnectionTransport;

pub const CONNECTION_RUNTIME_STALE_AFTER_SECONDS: u32 = 60;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetConnectionRuntimeEpochResponse {
    /// Zero means this node has never acquired a runtime lease.
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub epoch: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeRequest {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub instance_id: String,
    /// Pin the initial epoch read across retries; never refresh after a conflict.
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub expected_epoch: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginConnectionRuntimeResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionRuntimeState {
    AwaitingReport,
    Disabled,
    Applying,
    Applied,
    Error,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionRuntimeReason {
    None,
    Disabled,
    InvalidConfiguration,
    ProfileUnavailable,
    BindFailed,
    ResolveFailed,
    TransportFailed,
    AuthenticationFailed,
    ProbeFailed,
    LeaseSuperseded,
    LeaseExpired,
    ShuttingDown,
    InternalError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionListenerState {
    Bound,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionNeighborState {
    Authenticated,
    Waiting,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionListenerObservation {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub listener_id: String,
    pub state: ConnectionListenerState,
    pub reason: ConnectionRuntimeReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionNeighborObservation {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub peer_node_id: String,
    pub transport: ConnectionTransport,
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub initiator_node_id: String,
    #[schema(
        required = true,
        min_length = 1,
        max_length = 64,
        pattern = "^[A-Za-z0-9._-]+$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub listener_id: Option<String>,
    #[schema(
        required = true,
        min_length = 1,
        max_length = 64,
        pattern = "^[A-Za-z0-9._-]+$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub candidate_id: Option<String>,
    pub state: ConnectionNeighborState,
    /// Sanitized peer-local outcome, independent of global reconcile state.
    pub reason: ConnectionRuntimeReason,
    /// A recent authenticated control-nonce probe, not merely a control heartbeat.
    pub probe_fresh: bool,
    #[schema(required = true, format = Int64, minimum = 0, maximum = 4294967295_u64)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub probe_rtt_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportConnectionRuntimeRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub sequence: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub state: ConnectionRuntimeState,
    pub reason: ConnectionRuntimeReason,
    #[schema(max_items = 128)]
    pub listeners: Vec<ConnectionListenerObservation>,
    #[schema(max_items = 256)]
    pub peers: Vec<ConnectionNeighborObservation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportConnectionRuntimeResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub sequence: u32,
    pub reported_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRuntimeObservation {
    pub node_id: String,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub sequence: u32,
    #[schema(required = true, format = Int64, minimum = 1, maximum = 4294967295_u64)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub revision: Option<u32>,
    #[schema(
        required = true,
        min_length = 64,
        max_length = 64,
        pattern = "^[0-9a-f]{64}$"
    )]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub digest: Option<String>,
    pub state: ConnectionRuntimeState,
    pub reason: ConnectionRuntimeReason,
    #[schema(max_items = 128)]
    pub listeners: Vec<ConnectionListenerObservation>,
    #[schema(max_items = 256)]
    pub peers: Vec<ConnectionNeighborObservation>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub reported_at: Option<DateTime<Utc>>,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub stale_after_seconds: u32,
    /// Matches the current node-scoped revision/digest and node is active.
    pub current: bool,
    /// Not current, never reported, or receive age >= stale_after_seconds.
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRuntimesSnapshot {
    pub nodes: Vec<ConnectionRuntimeObservation>,
}
