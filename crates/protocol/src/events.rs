//! The SSE response is a byte stream of these JSON data objects, not one JSON
//! response. Management supplies its existing Config/Entity/Progress fragments.
use crate::{
    history::ConfigurationChangeCause,
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
        #[serde(deserialize_with = "meshway_serde::unique_map")]
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
        cause: ConfigurationChangeCause,
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
    #[serde(rename = "observation.removed")]
    ObservationRemoved {
        #[serde(rename = "subscriptionId")]
        subscription_id: meshway_model::ObservationSubscriptionId,
    },
    #[serde(rename = "sync.reset")]
    Reset { reason: ResetReason },
    #[serde(rename = "stream.closed")]
    Closed { reason: ClosedReason },
}
crate::object::wire_object! { #[derive(Clone, Debug)] pub struct EntityRemovals {
    pub nodes: Vec<NodeId>,
    pub members: Vec<MembershipId>,
} }

crate::object::wire_object! { #[derive(Clone, Debug)] pub struct ConfigurationNotice {
    pub network_id: NetworkId,
    pub previous_revision: Revision,
    pub revision: Revision,
    pub changed_at: DateTime<Utc>,
    pub cause: ConfigurationChangeCause,
    pub actor: EventActor,
    pub request_id: Option<RequestId>,
    pub changed_pointers: Vec<String>,
    pub changes_complete: bool,
    pub source_changed: bool,
} }
crate::object::wire_tagged! { #[derive(Clone, Debug)] pub enum EventActor {
    User {
        #[serde(rename = "userId")]
        user_id: UserId,
    },
    Node {
        #[serde(rename = "nodeId")]
        node_id: NodeId,
    },
    System,
    Redacted,
} }
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

// One generic declaration owns wire fields and variant names. Concrete roots
// give generators stable public types without copying the event contract.
macro_rules! event_root {
    ($name:ident, $configuration:ty, $entities:ty) => {
        #[derive(Clone, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub StreamEvent<$configuration, $entities, crate::live::ConfigurationApplyProgress>);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                StreamEvent::deserialize(crate::object::MapOnly(d)).map(Self)
            }
        }
        impl utoipa::PartialSchema for $name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                let mut schema = <StreamEvent<$configuration, $entities, crate::live::ConfigurationApplyProgress> as utoipa::PartialSchema>::schema();
                close_event_variants(&mut schema);
                schema
            }
        }
        impl ToSchema for $name {
            fn schemas(schemas: &mut Vec<(String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>)>) {
                let start = schemas.len();
                <StreamEvent<$configuration, $entities, crate::live::ConfigurationApplyProgress> as ToSchema>::schemas(schemas);
                schemas.push((<$configuration as ToSchema>::name().into_owned(), <$configuration as utoipa::PartialSchema>::schema()));
                <$configuration as ToSchema>::schemas(schemas);
                schemas.push((<$entities as ToSchema>::name().into_owned(), <$entities as utoipa::PartialSchema>::schema()));
                <$entities as ToSchema>::schemas(schemas);
                schemas.push((<crate::live::ConfigurationApplyProgress as ToSchema>::name().into_owned(), <crate::live::ConfigurationApplyProgress as utoipa::PartialSchema>::schema()));
                <crate::live::ConfigurationApplyProgress as ToSchema>::schemas(schemas);
                // Utoipa's internally tagged derives omit deny_unknown_fields
                // on their inline payload branches. Preserve the declared
                // closed object semantics, also for referenced State unions.
                for (_, schema) in &mut schemas[start..] { close_event_variants(schema); }
            }
        }
    };
}
fn close_event_variants(schema: &mut utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>) {
    use utoipa::openapi::{
        RefOr,
        schema::{AdditionalProperties, Schema},
    };
    match schema {
        RefOr::T(Schema::OneOf(union)) => {
            for item in &mut union.items {
                close_event_variants(item);
            }
        }
        RefOr::T(Schema::Object(object))
            if !object.properties.is_empty() && object.additional_properties.is_none() =>
        {
            object.additional_properties = Some(Box::new(AdditionalProperties::FreeForm(false)));
        }
        _ => {}
    }
}
event_root!(
    UserEvent,
    crate::live::UserConfigurationView,
    crate::live::UserEntitiesView
);
event_root!(
    NodeEvent,
    crate::live::NodeConfigurationView,
    crate::live::NodeEntitiesView
);
