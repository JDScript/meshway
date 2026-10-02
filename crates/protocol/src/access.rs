//! Exact compiled path authorization; no source policy lives in node DTOs.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
pub const MAX_MESH_ACCESS_GRANTS: usize = 128;
pub const MAX_MESH_ACCESS_PATH_NODES: usize = 8;
pub const MAX_DATAPLANE_IDENTITIES: usize = 64;
pub const MAX_DATAPLANE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOURCE_JSONC_BYTES: usize = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAccessGrant {
    #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9._-]+$")]
    pub id: String,
    /// Exact ordered full path, including source and destination. No repeats.
    #[schema(min_items = 2, max_items = 8)]
    pub path: Vec<String>,
}

/// Exact paths authorized by the current compiler.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshPathAuthorization {
    #[schema(max_items = 128)]
    pub grants: Vec<MeshAccessGrant>,
}
