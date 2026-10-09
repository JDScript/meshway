//! Permission-specific snapshots. These are read-only projections, never
//! installation payloads or writable configuration documents.
use crate::object::{wire_object, wire_tagged};
use meshway_configuration::{ConfigurationView, NodeSettingsView};
use meshway_model::{
    MemberSummary, NetmapRef, MembershipId, NetworkId, NetworkSummary, NodeId, Revision,
};
use std::collections::BTreeMap;

wire_tagged! { #[derive(Clone, PartialEq)]
/// Permission-specific desired configuration snapshot for an authenticated network user.
pub enum UserConfigurationView {
    /// Expose the complete redacted network configuration to an authorized manager.
    Manager {
        /// Network that owns this configuration snapshot.
        #[serde(rename="networkId")] network_id: NetworkId,
        /// Exact desired source revision represented by the configuration.
        revision: Revision,
        /// Complete network settings without secret material or sensitive header values.
        configuration: Box<ConfigurationView>
    },
    /// Expose only the member's permitted node settings.
    Member {
        /// Network that owns this configuration snapshot.
        #[serde(rename="networkId")] network_id: NetworkId,
        /// Exact desired source revision represented by the node settings.
        revision: Revision,
        /// Permitted node IDs mapped to their redacted desired settings.
        #[serde(deserialize_with="meshway_serde::unique_map")] nodes: BTreeMap<NodeId, NodeSettingsView>
    },
} }
wire_object! { #[derive(Clone, PartialEq)]
/// User-visible network, node and membership inventory under current permissions.
pub struct UserEntitiesView {
    /// Network summary and currently available actions.
    pub network: NetworkSummary,
    /// Visible node inventory keyed by node ID.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, crate::NodeInventoryView>,
    /// Visible member summaries keyed by stable network membership ID.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub members: BTreeMap<MembershipId, MemberSummary>,
} }

meshway_serde::string_enum! {
/// Whether progress covers the entire original apply target set or a permission-filtered subset.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ProgressScope {
    /// Include all original targets for an authorized network manager.
    Full,
    /// Include only targets visible to the caller, excluding hidden nodes from counts.
    Partial,
}
}
meshway_serde::string_enum! {
/// Last accepted result for an original configuration target, independent of current freshness.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationPhase {
    /// No accepted result has been recorded for this target.
    Pending,
    /// The node reported private persistence of the complete target.
    Saved,
    /// The node reported installation of its required resources; this alone does not prove business connectivity.
    Applied,
    /// The node reported a failed save or installation step.
    Failed,
}
}
wire_object! { #[derive(Clone, PartialEq)]
/// Historical application result and independent freshness for one original node target.
pub struct ConfigurationNodeProgress {
    /// Netmap of this node current after the original configuration apply.
    pub target: NetmapRef,
    /// Last result for the original target, including historical instances.
    pub phase: ApplicationPhase,
    /// Instance that submitted the last accepted result; null when no receipt exists.
    pub instance_id: Option<meshway_model::InstanceId>,
    /// Installation attempt identified by the last accepted receipt; null when no receipt exists.
    pub attempt: Option<meshway_model::Version>,
    /// Receipt sequence within that attempt; null when no receipt exists.
    pub sequence: Option<meshway_model::Sequence>,
    /// UTC time the Server received that result, rather than its observation time; null when no receipt exists.
    pub received_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Independent of historical result; never implies data-plane health.
    pub freshness: crate::execution::ReceiptFreshness,
} }
wire_object! { #[derive(Clone, PartialEq)]
/// Counts of original visible targets by last accepted phase, without interpreting historical results as health.
pub struct ConfigurationProgressSummary {
    /// Counts of the original targets' last accepted phase, not online/healthy
    /// nodes. Inspect each node's independent freshness before presenting it as
    /// currently applied; expired/superseded results remain historical evidence.
    pub target_count: u32,
    /// Original visible targets with no accepted result.
    pub pending: u32,
    /// Original visible targets whose last accepted result is saved.
    pub saved: u32,
    /// Original visible targets whose last accepted result is applied, even if subsequently expired or superseded.
    pub applied: u32,
    /// Original visible targets whose last accepted result is failed.
    pub failed: u32,
} }
wire_object! { #[derive(Clone, PartialEq)]
/// Application progress for a source revision's fixed original target set, filtered by current caller permissions.
pub struct ConfigurationApplyProgress {
    /// Network whose apply progress is reported.
    pub network_id: NetworkId,
    /// Source revision that originally established the reported targets.
    pub revision: Revision,
    /// Whether counts and node details cover all original targets or only the visible subset.
    pub scope: ProgressScope,
    /// Phase totals over exactly the nodes included in this projection.
    pub summary: ConfigurationProgressSummary,
    /// Original visible node targets and their last results, unaffected by later targets added by demands or refreshes.
    #[serde(deserialize_with="meshway_serde::unique_map")] pub nodes: BTreeMap<NodeId, ConfigurationNodeProgress>,
} }
