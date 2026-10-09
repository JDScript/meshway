//! The part of a network configuration one Node needs to run the shared
//! compiler for every pair it terminates or relays.
use crate::{
    ConfigurationView, SubjectContext,
    execution::{RuleSetContent, TrustedNode},
    object::{object, unique_map},
};
use meshway_model::NodeId;
use std::collections::BTreeMap;

object! {
/// A monotone subset of the full configuration: every rule is an unchanged
/// full-configuration rule in its original relative order, and every Node
/// declaration is the existing redacted `NodeSettingsView` of a visible Node.
pub struct ProjectedConfiguration {
    /// Redacted configuration restricted to visible Nodes and the rules that can affect them.
    pub configuration: ConfigurationView,
    /// User bindings of visible User-identity Nodes.
    pub subjects: SubjectContext,
    /// Trusted keys and authenticated capabilities of visible registered Nodes.
    #[serde(deserialize_with = "unique_map")]
    pub inventory: BTreeMap<NodeId, TrustedNode>,
    /// Resolved remote rule-set contents referenced by egress rules.
    #[serde(deserialize_with = "unique_map")]
    pub rule_sets: BTreeMap<String, RuleSetContent>,
    /// Persisted profile selections of visible Nodes.
    #[serde(deserialize_with = "unique_map")]
    pub selected_profiles: BTreeMap<NodeId, String>,
    /// Persisted selection versions of visible Nodes; each switch increments one.
    #[serde(deserialize_with = "unique_map")]
    pub selection_versions: BTreeMap<NodeId, u64> [value_type = BTreeMap<NodeId, meshway_model::Version>],
} }
