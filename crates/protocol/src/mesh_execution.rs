//! Stable node execution contract. Source policy and compiler indexes never cross
//! this boundary. V1 executes IPv4 Mesh only, with one exact path per flow.
use crate::{ConnectionPeer, ManagedVlessMaterial, MeshNetwork, MeshNode, NodeConnections};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

pub const MESH_EXECUTION_VERSION: u32 = 1;
pub const MAX_MESH_EXECUTION_PATHS: usize = 128;
pub const MAX_MESH_EXECUTION_MATERIALS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicySourceMode {
    LegacyV2,
    MeshV3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MeshExecutionCapability {
    Ipv4ExactPathV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub epoch: u32,
    pub instance_id: String,
    #[schema(max_items = 1)]
    pub capabilities: Vec<MeshExecutionCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionPath {
    #[schema(min_items = 2, max_items = 8)]
    pub nodes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeMeshExecution {
    #[schema(format = Int64, minimum = 1, maximum = 1)]
    pub schema_version: u32,
    pub network_id: String,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub node_id: String,
    pub required_capability: MeshExecutionCapability,
    pub network: MeshNetwork,
    pub node: MeshNode,
    #[schema(max_items = 1023)]
    pub peers: Vec<MeshNode>,
    pub valid: bool,
    #[schema(max_items = 128)]
    pub paths: Vec<MeshExecutionPath>,
    #[schema(max_items = 64)]
    pub identities: Vec<ConnectionPeer>,
    pub connections: NodeConnections,
    #[schema(max_items = 64)]
    pub materials: Vec<ManagedVlessMaterial>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionSnapshot {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub payload: NodeMeshExecution,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshExecutionResponse {
    pub source_mode: PolicySourceMode,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(required = true)]
    #[serde(deserialize_with = "crate::deserialize_required_nullable")]
    pub snapshot: Option<MeshExecutionSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeMeshExecutionRequest {
    pub runtime: MeshExecutionRequest,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeMeshExecutionResponse {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub digest: String,
    pub saved: bool,
}

pub fn mesh_execution_digest(payload: &NodeMeshExecution) -> Result<String, serde_json::Error> {
    Ok(Sha256::digest(crate::canonical_json(payload)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
