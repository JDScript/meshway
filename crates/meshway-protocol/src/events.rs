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
/// One SSE data frame containing a permission-filtered snapshot, update, or stream lifecycle notice.
pub enum StreamEvent<C, E, P> {
    /// First frame of an initial synchronization batch, containing the visible configuration.
    #[serde(rename = "config.snapshot")]
    ConfigSnapshot {
        /// Identifier shared by all snapshots and the ready frame in this batch.
        batch: String,
        /// Correlation ID of the HTTP request carrying this stream, as in a
        /// GET response and the `x-request-id` header.
        #[serde(rename = "requestId")]
        request_id: RequestId,
        /// Current configuration projected for the authenticated user or Node.
        configuration: C,
        /// Visible configuration application progress when available; current Server snapshots include it.
        progress: Option<P>,
    },
    /// Entity inventory frame of the initial synchronization batch.
    #[serde(rename = "entities.snapshot")]
    EntitiesSnapshot {
        /// Identifier shared with this batch's configuration, state, and ready frames.
        batch: String,
        /// Complete entity inventory within the caller's authorized scope.
        entities: E,
    },
    /// State frame of the initial synchronization batch, retaining original report freshness.
    #[serde(rename = "state.snapshot")]
    StateSnapshot {
        /// Identifier shared with this batch's configuration, entities, and ready frames.
        batch: String,
        /// Current permission-filtered operational state.
        state: StateView,
    },
    /// Commit marker allowing the client to apply a complete initial snapshot batch.
    #[serde(rename = "sync.ready")]
    Ready {
        /// Identifier of the completed snapshot batch.
        batch: String,
        /// Resume cursor to save only after the entire batch has been validated and applied.
        cursor: String,
        /// Expected interval between idle heartbeat comments, in milliseconds.
        #[serde(rename = "heartbeatIntervalMs")]
        heartbeat_interval_ms: u32,
        /// Read-idle timeout in milliseconds after which the client should reconnect.
        #[serde(rename = "disconnectAfterMs")]
        disconnect_after_ms: u32,
    },
    /// Incremental operational state projection following the previously applied stream cursor.
    #[serde(rename = "state.delta")]
    StateDelta {
        /// Epoch identifying the state continuity domain for this update.
        #[serde(rename = "stateEpoch")]
        state_epoch: StateEpoch,
        /// Projection time, not a new measurement timestamp. Current Server
        /// frames include these metadata fields; optionality permits consumers
        /// to apply previously retained deltas without inventing values.
        #[serde(rename = "asOf", skip_serializing_if = "Option::is_none")]
        as_of: Option<DateTime<Utc>>,
        /// Network committed-change sequence; omitted only in retained deltas lacking this metadata.
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence: Option<meshway_model::Sequence>,
        /// Configuration revision used for this projection; omitted in retained deltas lacking this metadata.
        #[serde(
            rename = "configurationRevision",
            skip_serializing_if = "Option::is_none"
        )]
        configuration_revision: Option<Revision>,
        /// Cursor of the preceding applied stream event, including non-state notices.
        #[serde(rename = "baseCursor")]
        base_cursor: String,
        /// Resume cursor to save after this delta has been validated and applied.
        cursor: String,
        /// Complete replacement state for each updated visible node.
        #[serde(deserialize_with = "meshway_serde::unique_map")]
        updated: BTreeMap<NodeId, NodeStateView>,
        /// Node identifiers to remove from the cached state view.
        removed: Vec<NodeId>,
        /// Replacement collection of currently visible aggregated links.
        links: Vec<crate::state::AggregatedLink>,
        /// Replacement collection of currently visible aggregated paths.
        paths: Vec<crate::state::AggregatedPath>,
    },
    /// Entity replacements and explicit removals following the previously applied stream cursor.
    #[serde(rename = "entities.delta")]
    EntitiesDelta {
        /// Cursor of the preceding applied stream event, including configuration and resource notices.
        #[serde(rename = "baseCursor")]
        base_cursor: String,
        /// Resume cursor to save after this delta has been validated and applied.
        cursor: String,
        /// Updated entity fragments that replace their corresponding cached entries.
        updated: E,
        /// Explicit entity identifiers to remove from the cached inventory.
        removed: EntityRemovals,
    },
    /// Notification of a committed configuration revision, independent of Node application progress.
    #[serde(rename = "configuration.changed")]
    ConfigurationChanged {
        /// Network whose saved configuration changed.
        #[serde(rename = "networkId")]
        network_id: NetworkId,
        /// Desired configuration revision immediately before the commit.
        #[serde(rename = "previousRevision")]
        previous_revision: Revision,
        /// Newly committed desired configuration revision.
        revision: Revision,
        /// Time at which the configuration change was committed.
        #[serde(rename = "changedAt")]
        changed_at: DateTime<Utc>,
        /// Primary trigger for the configuration change.
        cause: ConfigurationChangeCause,
        /// Committing actor, redacted for users without management scope and for Nodes.
        actor: EventActor,
        /// Committing request's correlation ID, or null when withheld from the subscriber.
        #[serde(rename = "requestId")]
        request_id: Option<RequestId>,
        /// Visible changed JSON Pointers, provided as invalidation hints rather than a complete dependency analysis.
        #[serde(rename = "changedPointers")]
        changed_pointers: Vec<String>,
        /// Whether the changed-pointer list is complete rather than redacted or truncated.
        #[serde(rename = "changesComplete")]
        changes_complete: bool,
        /// Whether the original configuration source text changed.
        #[serde(rename = "sourceChanged")]
        source_changed: bool,
    },
    /// Configuration application progress using the same projection as the progress GET operation.
    #[serde(rename = "configuration.progress")]
    ConfigurationProgress {
        /// Updated application progress for the fixed configuration submission target.
        progress: P,
    },
    /// A manager must re-read the normal resource GET view in this network.
    /// This notice carries no source URL, material or unscoped artifact data.
    #[serde(rename = "resources.changed")]
    ResourcesChanged {
        /// Network whose authorized resource view should be reread.
        #[serde(rename = "networkId")]
        network_id: NetworkId,
        /// Configuration revision associated with the resource change notification.
        revision: Revision,
    },
    /// Subscriber-specific observation task status update.
    #[serde(rename = "observation.status")]
    ObservationStatus {
        /// Observation task with this subscription's current status and execution context.
        observation: Box<ObservationTask>,
    },
    /// Subscriber-specific observation update containing actual measurement results.
    #[serde(rename = "observation.sample")]
    ObservationSample {
        /// Observation task with authorized samples and their original measurement context.
        observation: Box<ObservationTask>,
    },
    /// Notification that a terminated observation subscription has left the retained view.
    #[serde(rename = "observation.removed")]
    ObservationRemoved {
        /// Subscription whose cached observation view should be removed.
        #[serde(rename = "subscriptionId")]
        subscription_id: meshway_model::ObservationSubscriptionId,
    },
    /// Notification that incremental continuity was lost and a new full batch is required.
    #[serde(rename = "sync.reset")]
    Reset {
        /// Reason the previous cursor and cached view can no longer continue incrementally.
        reason: ResetReason,
    },
    /// Final stream notice before closure when an explicit reason can be delivered.
    #[serde(rename = "stream.closed")]
    Closed {
        /// Authentication, lease, or shutdown condition that ended the stream.
        reason: ClosedReason,
    },
}
crate::object::wire_object! { #[derive(Clone, Debug)]
/// Explicit entity removals applied separately from entity replacements.
pub struct EntityRemovals {
    /// Node identifiers to remove from the entity inventory.
    pub nodes: Vec<NodeId>,
    /// Membership identifiers to remove from the entity inventory.
    pub members: Vec<MembershipId>,
} }

crate::object::wire_object! { #[derive(Clone, Debug)]
/// Configuration commit metadata projected into a subscriber's change notification.
pub struct ConfigurationNotice {
    /// Network whose saved configuration changed.
    pub network_id: NetworkId,
    /// Desired configuration revision immediately before the commit.
    pub previous_revision: Revision,
    /// Newly committed desired configuration revision.
    pub revision: Revision,
    /// Time at which the configuration change was committed.
    pub changed_at: DateTime<Utc>,
    /// Primary trigger for the configuration change.
    pub cause: ConfigurationChangeCause,
    /// Committing actor, redacted when the subscriber lacks management scope.
    pub actor: EventActor,
    /// Committing request's correlation ID, or null when withheld from the subscriber.
    pub request_id: Option<RequestId>,
    /// Visible changed JSON Pointers used as invalidation hints.
    pub changed_pointers: Vec<String>,
    /// Whether the changed-pointer list is complete rather than redacted or truncated.
    pub changes_complete: bool,
    /// Whether the original configuration source text changed.
    pub source_changed: bool,
} }
crate::object::wire_tagged! { #[derive(Clone, Debug)]
/// Actor reference filtered for the subscriber's current disclosure permissions.
pub enum EventActor {
    /// Authenticated user responsible for the change.
    User {
        /// Stable account identifier of the acting user.
        #[serde(rename = "userId")]
        user_id: UserId,
    },
    /// Authenticated Node responsible for the change.
    Node {
        /// Stable identifier of the acting Node.
        #[serde(rename = "nodeId")]
        node_id: NodeId,
    },
    /// Server-originated change without a user or Node actor.
    System,
    /// Actor identity withheld because the subscriber lacks management scope.
    Redacted,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Reason a stream must replace its cached view through full synchronization.
pub enum ResetReason {
    /// Requested cursor is outside the retained replay window.
    CursorTooOld,
    /// Current authorization or disclosure scope differs from the cached view.
    ViewChanged,
    /// Buffered events exceeded the stream's bounded delivery capacity.
    Backpressure,
    /// State continuity epoch differs from the cached view.
    EpochChanged,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Explicit reason for ending an event stream before EOF.
pub enum ClosedReason {
    /// Session or current authentication eligibility is no longer valid.
    SessionExpired,
    /// User no longer has an active membership in the subscribed network.
    MembershipRevoked,
    /// Node instance or execution eligibility is no longer valid.
    NodeLeaseExpired,
    /// Server is shutting down normally.
    ServerShutdown,
}
}

// One generic declaration owns wire fields and variant names. Concrete roots
// give generators stable public types without copying the event contract.
macro_rules! event_root {
    ($name:ident, $configuration:ty, $entities:ty) => {
        #[derive(Clone, Serialize)]
        #[serde(transparent)]
        /// Concrete SSE frame using the caller-specific configuration and entity views.
        pub struct $name(
            /// Permission-filtered event serialized directly as the SSE data object.
            pub StreamEvent<$configuration, $entities, crate::live::ConfigurationApplyProgress>
        );

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
