use meshway_policy::v3::{Diagnostic, NetworkPolicy};
use meshway_protocol::PolicySourceMode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveNetworkPolicyRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    /// Explicit replacement/activation; not an assertion of equivalent migration.
    pub policy: NetworkPolicy,
    #[serde(default)]
    pub source_jsonc: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicySnapshot {
    #[schema(min_length = 32, max_length = 32, pattern = "^[0-9a-f]{32}$")]
    pub network_id: String,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub source_mode: PolicySourceMode,
    #[schema(required = true)]
    pub policy: Option<NetworkPolicy>,
    #[schema(required = true)]
    pub source_jsonc: Option<String>,
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Proposal only, never automatically activated.
    #[schema(required = true)]
    pub migration_proposal: Option<NetworkPolicy>,
}
