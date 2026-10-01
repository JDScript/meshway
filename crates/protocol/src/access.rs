//! Explicit directional authorization for complete mesh IPv4 paths. Grants
//! authorize every IPv4 packet type between the two assigned endpoint addresses;
//! they never authorize Internet access, IPv6, reverse paths or subpaths.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{ConnectionPeer, NodeConfiguration, NodeConnections, PolicyDiagnostic};

pub const MAX_MESH_ACCESS_BYTES: usize = 128 * 1024;
pub const MAX_MESH_ACCESS_GRANTS: usize = 128;
pub const MAX_MESH_ACCESS_PATH_NODES: usize = 8;
pub const MAX_DATAPLANE_IDENTITIES: usize = 64;
pub const DATAPLANE_SCHEMA_VERSION: u32 = 1;
pub const MAX_DATAPLANE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessGrant {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    /// Exact ordered full path, including source and destination. No repeats.
    #[schema(min_items = 2, max_items = 8)]
    pub path: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessPolicy {
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
    pub valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMeshAccessRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    pub policy: MeshAccessPolicy,
}

/// One coherent global revision, projected for the authenticated node. Identities
/// include E2E peers and confer no carrier adjacency. Invalid access withdraws
/// every effective grant. More than 64 union identities rejects the request.
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
    pub access_policy: MeshAccessPolicy,
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
