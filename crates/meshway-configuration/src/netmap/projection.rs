//! The part of a network configuration one Node needs to run the shared
//! compiler for every pair it terminates or relays.
use crate::{
    ConfigurationView, SubjectContext,
    execution::TrustedNode,
    object::{object, unique_map},
};
use meshway_model::NodeId;
use std::collections::BTreeMap;

object! {
/// One rule-set artifact a netmap depends on. Nodes fetch the bytes from
/// `GET /agent-api/v1/artifacts/{digest}` and check length and digest.
pub struct RuleSetRef {
    /// `sha256:` and the lowercase hex SHA-256 of the complete artifact.
    pub digest: meshway_rules::Digest [value_type = String, pattern = "^sha256:[0-9a-f]{64}$"],
    /// `MWRS` major format version of the artifact.
    pub format: u16,
    /// Artifact length in bytes.
    pub bytes: u32,
    /// Canonical members: names, keywords and merged address ranges.
    pub entries: u32,
} }

object! {
/// An exit the full configuration makes eligible (enabled with
/// `exitEnabled`) and the inner families its `exitFamilies` carry.
pub struct EligibleExit {
    /// Exit Node.
    pub node: NodeId,
    /// Configured families, ascending and unique.
    pub families: Vec<meshway_model::exit::ExitFamily>,
} }

object! {
/// A monotone subset of the full configuration: every rule is an unchanged
/// full-configuration rule in its original relative order, and every Node
/// declaration is the existing redacted `NodeSettingsView` of a visible Node.
/// `eligibleExits` is the one fact taken from outside the visible Nodes.
pub struct ProjectedConfiguration {
    /// Redacted configuration restricted to visible Nodes and the rules that can affect them.
    pub configuration: ConfigurationView,
    /// User bindings of visible User-identity Nodes.
    pub subjects: SubjectContext,
    /// Trusted keys and authenticated capabilities of visible registered Nodes.
    #[serde(deserialize_with = "unique_map")]
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    /// Content references of the rule sets retained egress rules name, keyed
    /// as in `match.ruleSets`; the content is fetched by digest.
    #[serde(deserialize_with = "unique_map")]
    pub rule_sets: BTreeMap<String, RuleSetRef>,
    /// Persisted profile selections of visible Nodes.
    #[serde(deserialize_with = "unique_map")]
    pub selected_profiles: BTreeMap<NodeId, String>,
    /// Persisted selection versions of visible Nodes; each switch increments one.
    #[serde(deserialize_with = "unique_map")]
    pub selection_versions: BTreeMap<NodeId, meshway_model::Version>,
    /// Eligible exits (enabled with `exitEnabled`) the retained routing
    /// names, visible or not, ascending by Node, with their configured
    /// families. Exit eligibility and the families an exit carries are read
    /// from this list only; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eligible_exits: Vec<EligibleExit>,
} }
