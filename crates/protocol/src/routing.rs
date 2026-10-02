//! Public Mesh address inventory, independent of policy source and execution.

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

/// Bounded diagnostics shared by address and DNS publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyDiagnostic {
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rule_id: Option<String>,
    pub field: String,
    pub message: String,
}
