use crate::{
    EventId, NodeId, Sequence,
    state::{ConfigurationRef, ProbeObservation},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ObservationEvent {
    pub event_id: EventId,
    pub configuration: ConfigurationRef,
    pub occurred_at: DateTime<Utc>,
    pub payload: ObservationPayload,
} }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(
    remote = "Self",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ObservationPayload {
    DialStarted {
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        #[serde(rename = "attemptId")]
        attempt_id: String,
        transport: String,
    },
    DialFailed {
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        #[serde(rename = "attemptId")]
        attempt_id: String,
        transport: String,
        reason: CloseReason,
        #[serde(rename = "durationUs")]
        duration_us: Option<Sequence>,
    },
    LinkOpened {
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        #[serde(rename = "linkId")]
        link_id: String,
        transport: String,
    },
    LinkClosed {
        #[serde(rename = "peerNodeId")]
        peer_node_id: NodeId,
        #[serde(rename = "linkId")]
        link_id: String,
        transport: String,
        reason: CloseReason,
    },
    ProbeCompleted {
        probe: ProbeObservation,
    },
    CollectionGap {
        dropped: Sequence,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
}
impl<'de> Deserialize<'de> for ObservationPayload {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::deserialize(meshway_serde::MapOnly(d))
    }
}
impl Serialize for ObservationPayload {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Idle,
    RemoteClosed,
    Timeout,
    Withdrawn,
    LocalFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MetricKind {
    Rtt,
    Traffic,
    ProbeLoss,
}

meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct TrafficWindow {
    pub tx_bytes: Sequence,
    pub rx_bytes: Sequence,
    pub counter_generation: Sequence,
} }
meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ProbeWindow {
    pub sent: u32,
    pub replied_within_deadline: u32,
    pub timeout: u32,
    pub pending: u32,
} }
meshway_serde::object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ObservationSample {
    pub measurement_id: EventId,
    /// Revisions replace a partial window rather than creating another sample.
    pub version: crate::Version,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub partial: bool,
    pub probe: Option<ProbeObservation>,
    pub traffic: Option<TrafficWindow>,
    pub probe_counts: Option<ProbeWindow>,
} }
