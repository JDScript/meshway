use crate::{
    WireSecret,
    object::{wire_object, wire_tagged},
};
use meshway_configuration::{
    ConfigurationChange, ConfigurationOperation, ConfigurationView, NetworkConfiguration,
};
use meshway_model::{NetworkId, NodeId, PublicationId, RequestId, Revision};

wire_tagged! {pub enum ConfigurationRequest {
    Document {#[serde(rename="expectedRevision")]expected_revision:Revision,#[serde(rename="requestId")]request_id:RequestId,#[serde(rename="sourceJsonc")]source_jsonc:WireSecret},
    Operations {#[serde(rename="expectedRevision")]expected_revision:Revision,#[serde(rename="requestId")]request_id:RequestId,ops:Vec<ConfigurationOperation>},
    Restore {#[serde(rename="expectedRevision")]expected_revision:Revision,#[serde(rename="requestId")]request_id:RequestId,#[serde(rename="restoreFrom")]restore_from:Revision,
        /// Omitted: the source version's initial resource bindings. Otherwise
        /// select this exact historical publication from the same source version.
        #[serde(default, rename="resourcePublication")]resource_publication:Option<PublicationId>},
} }
impl ConfigurationRequest {
    pub fn expected_revision(&self) -> Revision {
        match self {
            Self::Document {
                expected_revision, ..
            }
            | Self::Operations {
                expected_revision, ..
            }
            | Self::Restore {
                expected_revision, ..
            } => *expected_revision,
        }
    }
    pub fn request_id(&self) -> RequestId {
        match self {
            Self::Document { request_id, .. }
            | Self::Operations { request_id, .. }
            | Self::Restore { request_id, .. } => *request_id,
        }
    }
}
wire_object! {pub struct ConfigurationResponse {pub request_id:RequestId,pub network_id:NetworkId,pub revision:Revision,pub configuration:ConfigurationView} }
wire_object! {pub struct ConfigurationSourceResponse {pub request_id:RequestId,pub network_id:NetworkId,pub revision:Revision,pub source_jsonc:WireSecret,pub configuration:NetworkConfiguration} }
wire_object! {pub struct ConfigurationPreviewResponse {
    /// Transient canonical content checked by validate; no artifact/version reservation.
    #[serde(deserialize_with="meshway_serde::unique_map")]
    pub prepared_rule_sets: std::collections::BTreeMap<String,crate::resources::PreparedRuleSet>,
    pub unavailable_dependencies: Vec<meshway_configuration::ConfigurationIssue>,
    pub request_id:RequestId,pub base_revision:Revision,pub preview:ConfigurationView,
    pub change_count:u32,pub changes:Vec<ConfigurationChange>,pub source_changed:bool,
    pub deleted_node_ids:Vec<NodeId>,pub address_changed_node_ids:Vec<NodeId>,
} }
wire_object! {pub struct ConfigurationAppliedResponse {pub request_id:RequestId,pub network_id:NetworkId,pub revision:Revision,pub publication_ids:Vec<PublicationId>,pub target_count:u32,pub deleted_node_ids:Vec<NodeId>} }
wire_object! {pub struct CompareConfigurationRequest {pub from_revision:Revision,pub to_revision:Revision} }
wire_object! {pub struct ConfigurationComparison {pub request_id:RequestId,pub from_revision:Revision,pub to_revision:Revision,pub source_changed:bool,pub configuration_changed:bool,pub bindings_changed:bool,pub changes:Vec<ConfigurationChange>} }

#[derive(Default, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct ConfigurationRevisionQuery {
    pub revision: Option<Revision>,
}

wire_object! {pub struct ConfigurationSchemaResponse {
    pub request_id:RequestId,
    /// A complete JSON Schema 2020-12 document encoded as text for editor use.
    /// It contains definitions and constraints, never configuration values.
    pub schema_json:String,
} }
wire_object! {pub struct TransportCatalogEntry {
    pub protocol:meshway_configuration::TransportProtocol,
    pub outer_socket:meshway_configuration::OuterSocket,
    pub listener_schema:String,pub dialer_schema:String,
    /// Controller configuration support does not attest an Agent build's support.
    pub required_capability:String,
} }
wire_object! {pub struct TransportCatalogResponse {pub request_id:RequestId,pub items:Vec<TransportCatalogEntry>} }
