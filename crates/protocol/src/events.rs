//! The SSE response is a byte stream of these JSON data objects, not one JSON
//! response. Management supplies its existing Config/Entity/Progress fragments.
use crate::{
    observations::ObservationTask,
    state::{NodeStateView, StateView},
};
use chrono::{DateTime, Utc};
use meshway_model::{MembershipId, NetworkId, NodeId, RequestId, Revision, StateEpoch, UserId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum StreamEvent<C, E, P> {
    #[serde(rename = "config.snapshot")]
    ConfigSnapshot {
        batch: String,
        configuration: C,
        progress: Option<P>,
    },
    #[serde(rename = "entities.snapshot")]
    EntitiesSnapshot { batch: String, entities: E },
    #[serde(rename = "state.snapshot")]
    StateSnapshot { batch: String, state: StateView },
    #[serde(rename = "sync.ready")]
    Ready {
        batch: String,
        cursor: String,
        #[serde(rename = "heartbeatIntervalMs")]
        heartbeat_interval_ms: u32,
        #[serde(rename = "disconnectAfterMs")]
        disconnect_after_ms: u32,
    },
    #[serde(rename = "state.delta")]
    StateDelta {
        #[serde(rename = "stateEpoch")]
        state_epoch: StateEpoch,
        #[serde(rename = "baseCursor")]
        base_cursor: String,
        cursor: String,
        updated: BTreeMap<NodeId, NodeStateView>,
        removed: Vec<NodeId>,
        links: Vec<crate::state::AggregatedLink>,
        paths: Vec<crate::state::AggregatedPath>,
    },
    #[serde(rename = "entities.delta")]
    EntitiesDelta {
        #[serde(rename = "baseCursor")]
        base_cursor: String,
        cursor: String,
        updated: E,
        removed: EntityRemovals,
    },
    #[serde(rename = "configuration.changed")]
    ConfigurationChanged {
        #[serde(rename = "networkId")]
        network_id: NetworkId,
        #[serde(rename = "previousRevision")]
        previous_revision: Revision,
        revision: Revision,
        #[serde(rename = "changedAt")]
        changed_at: DateTime<Utc>,
        cause: ConfigurationCause,
        actor: EventActor,
        #[serde(rename = "requestId")]
        request_id: Option<RequestId>,
        #[serde(rename = "changedPointers")]
        changed_pointers: Vec<String>,
        #[serde(rename = "changesComplete")]
        changes_complete: bool,
        #[serde(rename = "sourceChanged")]
        source_changed: bool,
    },
    #[serde(rename = "configuration.progress")]
    ConfigurationProgress { progress: P },
    #[serde(rename = "observation.status")]
    ObservationStatus { observation: Box<ObservationTask> },
    #[serde(rename = "observation.sample")]
    ObservationSample { observation: Box<ObservationTask> },
    #[serde(rename = "sync.reset")]
    Reset { reason: ResetReason },
    #[serde(rename = "stream.closed")]
    Closed { reason: ClosedReason },
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityRemovals {
    pub nodes: Vec<NodeId>,
    pub members: Vec<MembershipId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationNotice {
    pub network_id: NetworkId,
    pub previous_revision: Revision,
    pub revision: Revision,
    pub changed_at: DateTime<Utc>,
    pub cause: ConfigurationCause,
    pub actor: EventActor,
    pub request_id: Option<RequestId>,
    pub changed_pointers: Vec<String>,
    pub changes_complete: bool,
    pub source_changed: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationCause {
    Edit,
    Restore,
    Enrollment,
    NodeDeleted,
    EmailSync,
    MemberRemoved,
    IdentityTransition,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum EventActor {
    User {
        #[serde(rename = "userId")]
        user_id: UserId,
    },
    Node {
        #[serde(rename = "nodeId")]
        node_id: NodeId,
    },
    System,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetReason {
    CursorTooOld,
    ViewChanged,
    Backpressure,
    EpochChanged,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClosedReason {
    SessionExpired,
    MembershipRevoked,
    NodeLeaseExpired,
    ServerShutdown,
}
