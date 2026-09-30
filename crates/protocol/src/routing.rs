//! Logical routing intent and preview contracts. A route decision never grants
//! network access or asserts transport reachability; dataplane and ACLs are separate.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshNetwork {
    pub ipv4_prefix: String,
    pub ipv6_prefix: String,
}

/// Public active-node inventory; country is administrator-managed metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshNode {
    pub id: String,
    pub name: String,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub country: Option<String>,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutingPolicy {
    pub default_action: DefaultRouteAction,
    pub rules: Vec<RouteRule>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefaultRouteAction {
    Direct,
    #[default]
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    pub id: String,
    pub source: SourceSelector,
    pub destination_scope: DestinationScope,
    pub destination_any: Vec<DestinationSelector>,
    pub action: RouteAction,
}

/// Empty dimensions are wildcards; dimensions combine with AND, members with OR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSelector {
    pub node_ids: Vec<String>,
    pub countries: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DestinationScope {
    Any,
    Mesh,
    Internet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DestinationSelector {
    pub kind: DestinationSelectorKind,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DestinationSelectorKind {
    Node,
    DomainSuffix,
    Cidr,
    Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteAction {
    pub kind: RouteActionKind,
    pub via: Vec<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub exit_node_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouteActionKind {
    Route,
    Reject,
}

/// Internet country is a preview hint, not automatic geolocation. Mesh country
/// and source country always come from the active-node inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowQuery {
    pub source_node_id: String,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub destination_node_id: Option<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub destination_ip: Option<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub domain: Option<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub destination_country: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostic {
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rule_id: Option<String>,
    pub field: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteDecision {
    pub action: RouteDecisionAction,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rule_id: Option<String>,
    pub path: Vec<String>,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub exit_node_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionAction {
    Route,
    Reject,
    Invalid,
}
