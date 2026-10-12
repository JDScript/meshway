use crate::{
    WireSecret,
    object::{wire_object, wire_tagged},
};
use meshway_configuration::{
    ConfigurationChange, ConfigurationOperation, ConfigurationView, NetworkConfigurationInput,
};
use meshway_model::{NetworkId, NodeId, RequestId, Revision};

wire_tagged! {
/// Complete configuration edit or historical restore, checked against the current revision before commit.
pub enum ConfigurationRequest {
    /// Replace the configuration from a complete JSONC document.
    Document {
        /// Current network revision required by strict compare-and-swap validation.
        #[serde(rename="expectedRevision")]expected_revision:Revision,
        /// Apply idempotency key; reuse with a different request body returns `request_id_conflict`.
        #[serde(rename="requestId")]request_id:RequestId,
        /// Full JSONC source including comments and any secret material; handled as sensitive input.
        #[serde(rename="sourceJsonc")]source_jsonc:WireSecret
    },
    /// Apply ordered typed edits to the current JSONC baseline as one atomic change.
    Operations {
        /// Current network revision required by strict compare-and-swap validation.
        #[serde(rename="expectedRevision")]expected_revision:Revision,
        /// Apply idempotency key; reuse with a different request body returns `request_id_conflict`.
        #[serde(rename="requestId")]request_id:RequestId,
        /// Ordered edits, each separately authorized before validating and committing the complete target.
        ops:Vec<ConfigurationOperation>
    },
    /// Restore a historical configuration as a new revision under current identity and dependency checks.
    Restore {
        /// Current network revision required by strict compare-and-swap validation.
        #[serde(rename="expectedRevision")]expected_revision:Revision,
        /// Apply idempotency key; reuse with a different request body returns `request_id_conflict`.
        #[serde(rename="requestId")]request_id:RequestId,
        /// Historical source revision in the same network whose configuration is restored.
        #[serde(rename="restoreFrom")]restore_from:Revision,
        /// Omitted or `initial`: the source version's initial rule-set bindings;
        /// `refreshed`: its last refreshed bindings, or the initial ones when never refreshed.
        #[serde(default, rename="ruleSetBindings")]rule_set_bindings:Option<RuleSetBindingChoice>},
} }
meshway_model::string_enum! {
/// Which rule-set bindings of a source version a restore reuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum RuleSetBindingChoice {
    /// The bindings the version was committed with.
    Initial,
    /// The version's last refreshed bindings.
    Refreshed,
}
}
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
wire_object! {
/// Redacted configuration snapshot at an exact network source revision.
pub struct ConfigurationResponse {
    /// Correlation identifier for this HTTP request.
    pub request_id:RequestId,
    /// Network that owns the configuration.
    pub network_id:NetworkId,
    /// Exact source revision represented by this snapshot.
    pub revision:Revision,
    /// Desired configuration with credentials and sensitive header values removed.
    pub configuration:ConfigurationView
} }
wire_object! {
/// Editable source and canonical configuration for authorized full-document editing, with saved listener secrets withheld.
pub struct ConfigurationSourceResponse {
    /// Correlation identifier for this HTTP request.
    pub request_id:RequestId,
    /// Network that owns the configuration.
    pub network_id:NetworkId,
    /// Exact source revision represented by both document forms.
    pub revision:Revision,
    /// Saved JSONC text with comments and layout; listener `uuid`, `password` and `tls.privateKey` values it spells out read `{"retained": true}`. WebSocket header values stay as written.
    pub source_jsonc:WireSecret,
    /// Complete canonical configuration as an editable document: fixed defaults, assigned addresses and certificates explicit, every saved listener `uuid`, `password` and `tls.privateKey` as `{"retained": true}`. Submitted as `sourceJsonc` it keeps those values.
    pub configuration:NetworkConfigurationInput
} }
wire_object! {
/// One listener affected by a proposed configuration change.
pub struct ListenerChange {
    /// Node that owns the changed listener.
    pub node_id: NodeId,
    /// Node-local listener key used to locate the resource.
    pub key: String,
    /// Whether the listener is added, removed or changed, including planned material generation.
    pub kind: meshway_configuration::ChangeKind,
} }
// Which configuration a plan is computed over: the current version, or a
// draft given exactly as validate takes it (CAS against expectedRevision).
wire_tagged! {
/// Current configuration or a CAS-bound draft used for read-only path planning.
pub enum EvaluationConfiguration {
    /// Evaluate the current saved network configuration.
    Current,
    /// Evaluate a full JSONC draft without saving it or generating permanent materials.
    Document {
        /// Current revision the draft must match; a stale baseline returns `revision_conflict`.
        #[serde(rename="expectedRevision")]expected_revision:Revision,
        /// Complete sensitive JSONC draft to validate and evaluate.
        #[serde(rename="sourceJsonc")]source_jsonc:WireSecret
    },
    /// Evaluate ordered typed edits against the current baseline without committing them.
    Operations {
        /// Current revision the draft must match; a stale baseline returns `revision_conflict`.
        #[serde(rename="expectedRevision")]expected_revision:Revision,
        /// Ordered, individually authorized edits that form the evaluated target.
        ops:Vec<ConfigurationOperation>
    },
} }
// Where traffic from the source would go. A Mesh address resolves to its node;
// `internet` stands for a public destination no rule names.
wire_tagged! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Business destination to plan without inferring DNS context from an IP address.
pub enum EvaluationTarget {
    /// Plan traffic to a concrete Mesh node.
    Node {
        /// Destination node in the evaluated network configuration.
        #[serde(rename="nodeId")] node_id: NodeId
    },
    /// Plan traffic to a literal IP, resolving allocated Mesh addresses to their nodes.
    Address {
        /// Literal IPv4 or IPv6 destination address.
        address: String
    },
    /// Plan name-based routing without performing DNS resolution.
    Domain {
        /// Exact DNS name, normalized for matching against domain and label-suffix rules.
        name: String
    },
    /// Represent an unnamed public destination using Internet grants and the profile fallback.
    Internet,
} }
wire_object! {
/// Read-only path planning request for one source and destination.
pub struct EvaluateConfigurationRequest {
    /// Current configuration or a draft that must satisfy strict revision checks.
    pub configuration: EvaluationConfiguration,
    /// Source node, required to exist in the evaluated configuration.
    pub source: NodeId,
    /// Business destination used for access and routing evaluation.
    pub target: EvaluationTarget,
    /// Omitted: the source's current selection or default.
    #[serde(default)] pub profile: Option<String>,
} }
wire_object! {
/// Rule and path planning result, independent of live connectivity or installation state.
pub struct EvaluationResponse {
    /// Correlation identifier for this HTTP request.
    pub request_id: RequestId,
    /// The current version the plan is based on; a draft is evaluated as a
    /// change to it.
    pub revision: Revision,
    /// Whether the result evaluates proposed changes rather than the current saved configuration.
    pub draft: bool,
    /// Resolved destination, grants, routing decisions, planned paths and availability outcome.
    pub evaluation: meshway_configuration::evaluation::Evaluation,
} }
wire_object! {
/// Redacted, non-executable draft comparison that reserves no revision, address or permanent material.
pub struct ConfigurationPreviewResponse {
    /// Transient canonical content checked by validate; no artifact/version reservation.
    #[serde(deserialize_with="meshway_model::wire::unique_map")]
    pub prepared_rule_sets: std::collections::BTreeMap<String,crate::resources::PreparedRuleSet>,
    /// Retained references whose dependencies are revoked or inactive, without restoring their execution authority.
    pub unavailable_dependencies: Vec<meshway_configuration::ConfigurationIssue>,
    /// Accepted as written but probably unintended, e.g. a shadowed rule.
    pub warnings: Vec<meshway_configuration::ConfigurationIssue>,
    /// Material the author still has to complete or that awaits verification.
    /// Disjoint from `willGenerate`; preview is never executable.
    #[serde(default)]
    pub requirements: Vec<meshway_configuration::PreparationRequirement>,
    /// Values that apply will generate, after rechecking authorization and CAS.
    #[serde(default)]
    pub will_generate: Vec<meshway_configuration::PreparationRequirement>,
    /// Correlation identifier for this validation request.
    pub request_id:RequestId,
    /// Actual current revision used as the comparison baseline.
    pub base_revision:Revision,
    /// Redacted normalized target declaration, with pending automatic materials still absent.
    pub preview:ConfigurationView,
    /// Number of final effective configuration and binding differences, equal to the length of `changes`.
    pub change_count:u32,
    /// Complete secret-free summary of final differences, rather than an operation or edit count.
    pub changes:Vec<ConfigurationChange>,
    /// Whether the JSONC source text changes, including comments without effective value changes.
    pub source_changed:bool,
    /// Nodes that apply would permanently delete and revoke.
    pub deleted_node_ids:Vec<NodeId>,
    /// Nodes with desired Mesh address changes that retire old bindings and interrupt affected sessions.
    pub address_changed_node_ids:Vec<NodeId>,
    /// Nodes whose identity or its stable binding changes.
    pub identity_changed_node_ids:Vec<NodeId>,
    /// Listeners added, removed or changed, including material to be generated.
    pub changed_listeners:Vec<ListenerChange>,
    /// Nodes whose execution target may change: every node unless all changes
    /// are confined to node-local display fields (`hostname`, `remark`) and
    /// the bindings of unchanged values.
    pub affected_node_ids:Vec<NodeId>,
} }
wire_object! {
/// Committed configuration revision and its original execution targets, without asserting node installation.
pub struct ConfigurationAppliedResponse {
    /// Correlation identifier for the configuration apply request.
    pub request_id:RequestId,
    /// Network whose configuration was committed.
    pub network_id:NetworkId,
    /// Source revision resulting from the accepted apply.
    pub revision:Revision,
    /// Nodes whose netmap after this apply is the revision's progress target, counted once per node.
    pub target_count:u32,
    /// Nodes permanently removed and revoked by this commit.
    pub deleted_node_ids:Vec<NodeId>
} }
wire_object! {
/// Two saved source revisions in the requested network to compare.
pub struct CompareConfigurationRequest {
    /// Baseline revision for the comparison.
    pub from_revision:Revision,
    /// Target revision compared with the baseline.
    pub to_revision:Revision
} }
wire_object! {
/// Secret-free comparison of source text, canonical values and stable bindings between saved revisions.
pub struct ConfigurationComparison {
    /// Correlation identifier for this HTTP request.
    pub request_id:RequestId,
    /// Baseline source revision.
    pub from_revision:Revision,
    /// Target source revision.
    pub to_revision:Revision,
    /// Whether original JSONC text differs, including comments and formatting.
    pub source_changed:bool,
    /// Whether canonical configuration values differ.
    pub configuration_changed:bool,
    /// Whether stable user, node identity, listener or rule-set bindings differ.
    pub bindings_changed:bool,
    /// JSON Pointer differences for configuration values and stable bindings, without secret before/after values.
    pub changes:Vec<ConfigurationChange>
} }

/// Optional source revision selector for reading the complete configuration document.
#[derive(Default, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct ConfigurationRevisionQuery {
    /// Saved revision to read; omission selects the current revision.
    pub revision: Option<Revision>,
}

wire_object! {
/// Complete configuration schema document for validating and editing network declarations.
pub struct ConfigurationSchemaResponse {
    /// Correlation identifier for this HTTP request.
    pub request_id:RequestId,
    /// A complete JSON Schema 2020-12 document encoded as text for editor use.
    /// It contains definitions and constraints, never configuration values.
    pub schema_json:String,
} }
wire_object! {
/// Server-supported transport declaration and its required node runtime capability.
pub struct TransportCatalogEntry {
    /// Protocol wire name used by listener types, dialer keys and topology restrictions.
    pub protocol:meshway_configuration::TransportProtocol,
    /// Underlay socket family required by this transport, independent of the inner business protocol.
    pub outer_socket:meshway_configuration::OuterSocket,
    /// Definition name in getConfigurationSchema.schemaJson.$defs (not schema text).
    pub listener_schema:String,
    /// Definition name in getConfigurationSchema.schemaJson.$defs (not schema text).
    pub dialer_schema:String,
    /// Controller configuration support does not attest an Agent build's support.
    pub required_capability:String,
} }
wire_object! {
/// Supported transport declaration catalog, without asserting runtime availability on a node.
pub struct TransportCatalogResponse {
    /// Correlation identifier for this HTTP request.
    pub request_id:RequestId,
    /// Available protocol declarations and their schema definition names.
    pub items:Vec<TransportCatalogEntry>
} }
