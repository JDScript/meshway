//! Permission-specific snapshots. These are read-only projections, never
//! installation payloads or writable configuration documents.
use crate::object::{wire_object, wire_tagged};
use meshway_configuration::{ConfigurationView, NodeSettingsView};
use meshway_model::{
    ConfigurationRef, MemberSummary, MembershipId, NetworkId, NetworkSummary, NodeId, Revision,
};
use std::collections::BTreeMap;

wire_tagged! { #[derive(Clone, PartialEq)] pub enum UserConfigurationView {
    Manager { #[serde(rename="networkId")] network_id: NetworkId, revision: Revision, configuration: Box<ConfigurationView> },
    Member { #[serde(rename="networkId")] network_id: NetworkId, revision: Revision, #[serde(deserialize_with="meshway_serde::unique_map")] nodes: BTreeMap<NodeId, NodeSettingsView> },
} }
wire_object! { #[derive(Clone, PartialEq)] pub struct NodeConfigurationView {
    pub network_id: NetworkId,
    pub node_id: NodeId,
    /// Revision of the desired settings below, not the installable target.
    pub revision: Revision,
    pub settings: NodeSettingsView,
    /// Exact current execution target. Fetch materials with configuration/query.
    pub target: Option<ConfigurationRef>,
} }
wire_object! { #[derive(Clone, PartialEq)] pub struct UserEntitiesView {
    pub network: NetworkSummary,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, crate::NodeInventoryView>,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub members: BTreeMap<MembershipId, MemberSummary>,
} }
wire_object! { #[derive(Clone, PartialEq)] pub struct NodeEntitiesView {
    #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, crate::NodeInventoryView>,
} }

meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ProgressScope {
    Full,
    Partial,
}
}
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationPhase {
    Pending,
    Saved,
    Applied,
    Failed,
}
}
wire_object! { #[derive(Clone, PartialEq)] pub struct ConfigurationNodeProgress {
    pub target: ConfigurationRef,
    /// Last result for the original target, including historical instances.
    pub phase: ApplicationPhase,
    pub instance_id: Option<meshway_model::InstanceId>,
    pub attempt: Option<meshway_model::Version>,
    pub sequence: Option<meshway_model::Sequence>,
    pub received_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Independent of historical result; never implies data-plane health.
    pub freshness: crate::execution::ReceiptFreshness,
} }
wire_object! { #[derive(Clone, PartialEq)] pub struct ConfigurationProgressSummary {
    /// Counts of the original targets' last accepted phase, not online/healthy
    /// nodes. Inspect each node's independent freshness before presenting it as
    /// currently applied; expired/superseded results remain historical evidence.
    pub target_count: u32, pub pending: u32, pub saved: u32, pub applied: u32, pub failed: u32,
} }
wire_object! { #[derive(Clone, PartialEq)] pub struct ConfigurationApplyProgress {
    pub network_id: NetworkId, pub revision: Revision, pub scope: ProgressScope,
    pub summary: ConfigurationProgressSummary,
    #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, ConfigurationNodeProgress>,
} }
