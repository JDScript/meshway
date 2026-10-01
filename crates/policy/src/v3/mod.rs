//! Four-layer engine: connections, grants and Mesh are order-independent;
//! external routing is ordered first-match. Facts belong to one network snapshot.
mod engine;
mod index;
mod migration;
mod profiles;
mod rule_sets;
mod validation;
pub use index::{DestinationIndex, FlattenedPrefix};
pub use engine::*;
pub use migration::*;
pub use profiles::*;
pub use rule_sets::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;
pub use validation::validate;

pub const MAX_NODES: usize = 1024;
pub const MAX_RULES: usize = 128;
pub const MAX_PATH_NODES: usize = 8;
pub const MAX_SEARCH_STATES: usize = 100_000;
pub const MAX_COMPILED_PATHS: usize = 4_200_000;
pub const MAX_DESTINATIONS: usize = 16384;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
/// Flat planning / migration kernel. Product configuration uses `NetworkPolicy`
/// and `compile_network`; route schemes never change these shared three layers.
pub struct Policy {
    pub connections: Vec<ConnectionRule>,
    pub grants: Vec<Grant>,
    pub mesh: Vec<MeshRule>,
    pub exits: Vec<ExitRule>,
    #[serde(default)]
    pub defaults: Vec<DefaultExitRule>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAction {
    Allow,
    Block,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRule {
    pub id: String,
    pub a: String,
    pub b: String,
    pub action: ConnectionAction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: String,
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub bidirectional: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshRule {
    pub id: String,
    pub src: String,
    pub dst: String,
    pub via: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExitMatch {
    DomainSuffix(String),
    Cidr(String),
    RuleSet(String),
}
fn wildcard() -> String {
    "*".into()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitRule {
    pub id: String,
    #[serde(default = "wildcard")]
    pub src: String,
    pub destinations: Vec<ExitMatch>,
    pub target: ExitTarget,
    pub candidates: Vec<String>,
    pub via: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExitTarget {
    Nodes,
    Direct,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultExitRule {
    pub id: String,
    #[serde(default = "wildcard")]
    pub src: String,
    pub families: BTreeSet<AddressFamily>,
    pub candidates: Vec<String>,
    pub via: Vec<String>,
    pub locked: bool,
    pub target: ExitTarget,
}
pub(crate) fn default_rules(p: &Policy) -> Vec<DefaultExitRule> {
    let mut rules = p.defaults.clone();
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    rules
}
/// Listener and directional dial facts are independent from business direction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub tags: BTreeSet<String>,
    pub listener: bool,
    /// Controller-administered exit enablement, independent of relay duties.
    pub exit_enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dial {
    pub from: String,
    pub to: String,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub nodes: Vec<Node>,
    pub tags: BTreeSet<String>,
    pub reachable: BTreeSet<Dial>,
    pub rule_sets: BTreeMap<String, ParsedRuleSet>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub field: String,
    pub message: String,
}
pub(crate) fn matches(s: &str, n: &Node) -> bool {
    s == "*" || s.strip_prefix("node:") == Some(n.id.as_str()) || n.tags.contains(s)
}
pub(crate) fn specificity(s: &str) -> u8 {
    if s.starts_with("node:") {
        2
    } else if s.starts_with("tag:") {
        1
    } else {
        0
    }
}
pub(crate) fn allowed(p: &Policy, a: &Node, b: Option<&Node>) -> bool {
    p.grants.iter().any(|g| {
        let sm = |ss: &[String], n: &Node| ss.iter().any(|s| matches(s, n));
        let dm = |ss: &[String]| match b {
            Some(n) => sm(ss, n),
            None => ss.iter().any(|s| s == "autogroup:internet"),
        };
        (sm(&g.src, a) && dm(&g.dst))
            || (g.bidirectional && b.is_some_and(|b| sm(&g.dst, a) && sm(&g.src, b)))
    })
}
impl Policy {
    pub fn default_template() -> Self {
        Self {
            connections: vec![ConnectionRule {
                id: "default".into(),
                a: wildcard(),
                b: wildcard(),
                action: ConnectionAction::Allow,
            }],
            grants: vec![Grant {
                id: "members".into(),
                src: vec![wildcard()],
                dst: vec![wildcard()],
                bidirectional: false,
            }],
            ..Self::default()
        }
    }
}
