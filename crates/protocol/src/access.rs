//! Endpoint authorization source intent and the legacy-compatible exact path
//! projection consumed by agents.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{
    ConnectionPeer, FlowQuery, NodeConfiguration, NodeConnections, PolicyDiagnostic, RouteDecision,
};

pub const MAX_MESH_ACCESS_BYTES: usize = 128 * 1024;
pub const MAX_MESH_ACCESS_RULES: usize = 128;
pub const MAX_MESH_ACCESS_SELECTORS: usize = 32;
pub const MAX_MESH_ACCESS_GRANTS: usize = 128;
pub const MAX_MESH_ACCESS_PATH_NODES: usize = 8;
pub const MAX_DATAPLANE_IDENTITIES: usize = 64;
pub const DATAPLANE_SCHEMA_VERSION: u32 = 1;
pub const MAX_DATAPLANE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOURCE_JSONC_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessAction {
    Allow,
    #[default]
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessRule {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    #[schema(min_items = 1, max_items = 32)]
    pub source: Vec<String>,
    #[schema(min_items = 1, max_items = 32)]
    pub destination: Vec<String>,
    pub action: AccessAction,
    pub bidirectional: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessPolicy {
    pub default_action: AccessAction,
    #[schema(max_items = 128)]
    pub rules: Vec<MeshAccessRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessGrant {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    /// Exact ordered full path, including source and destination. No repeats.
    #[schema(min_items = 2, max_items = 8)]
    pub path: Vec<String>,
}

/// Internal compiled path authorization. Its JSON remains `{ "grants": [...] }`
/// so schema-version-1 NodeDataplane remains byte-contract compatible.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshPathAuthorization {
    #[schema(max_items = 128)]
    pub grants: Vec<MeshAccessGrant>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessSnapshot {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    /// Saved source intent, retained even when references become invalid.
    pub policy: MeshAccessPolicy,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub source_jsonc: Option<String>,
    pub valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMeshAccessRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub policy: MeshAccessPolicy,
    #[serde(default)]
    pub source_jsonc: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessMatchedDirection {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowAuthorizationAction {
    Allow,
    Reject,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowAuthorizationDecision {
    pub action: FlowAuthorizationAction,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub rule_id: Option<String>,
    #[schema(required = true, value_type = Option<String>, pattern = "^(forward|reverse)$")]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub matched_direction: Option<AccessMatchedDirection>,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewFlowRequest {
    pub flow: FlowQuery,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewFlowResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub valid: bool,
    pub authorization: FlowAuthorizationDecision,
    pub routing: RouteDecision,
    pub diagnostics: Vec<PolicyDiagnostic>,
    pub execution_supported: bool,
    pub execution_reason: String,
}

/// One coherent global revision, projected for the authenticated node.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeDataplane {
    #[schema(format = Int64, minimum = 1, maximum = 1)]
    pub schema_version: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub node_id: String,
    pub configuration: NodeConfiguration,
    pub connections: NodeConnections,
    pub access_policy: MeshPathAuthorization,
    pub access_valid: bool,
    #[schema(max_items = 64)]
    pub identities: Vec<ConnectionPeer>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentDataplaneSnapshot {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub payload: NodeDataplane,
}

pub fn dataplane_digest(payload: &NodeDataplane) -> Result<String, serde_json::Error> {
    Ok(Sha256::digest(crate::canonical_json(payload)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
