//! Authoritative wire DTOs and shared local credential/file primitives.

pub mod access;
pub mod accounts;
pub mod config;
pub mod connection_runtime;
pub mod connections;
pub mod device_login;
pub mod endpoint_discovery;
pub mod inventory;
pub mod latency_history;
pub mod local;
pub mod managed_transports;
pub mod mesh_execution;
pub mod network;
pub mod stun;
pub mod tags;
pub use access::*;
pub use config::*;
pub use connection_runtime::*;
pub use connections::*;
pub use device_login::*;
pub use endpoint_discovery::*;
pub use inventory::*;
pub use latency_history::*;
pub use managed_transports::*;
pub use mesh_execution::*;
pub use network::*;
pub use tags::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Overview {
    pub name: String,
    pub version: String,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub node_count: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub configuration_revision: u32,
}

/// Durable control-plane identity. Last contact is not tunnel health.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
    #[schema(required = true)]
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub country: Option<String>,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub configuration_revision: u32,
    #[schema(format = Int64, minimum = 0, maximum = 4294967295_u64)]
    pub acknowledged_revision: u32,
    pub agent_version: String,
    pub last_seen: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
    #[schema(required = true)]
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub revoked_at: Option<DateTime<Utc>>,
}

// A custom field deserializer makes serde reject a missing field before calling
// this function, while Option still accepts an explicitly present JSON null.
// Keep ordinary Option serialization: None must serialize as null, not omission.
fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeList {
    pub nodes: Vec<NodeSummary>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollmentTokenRequest {
    #[schema(minimum = 60, maximum = 86400)]
    pub expires_in_seconds: u32,
}

/// The token is revealed only once, in this creation response.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollmentTokenResponse {
    pub token_id: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeNodeRequest {
    #[schema(min_length = 1, max_length = 128)]
    pub node_id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeNodeResponse {
    pub node: NodeSummary,
}

/// Persist the proposed random credential before sending this request. Retry
/// with the same enrollment token, credential, name and enrollment version.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnrollAgentRequest {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub node_credential: String,
    #[schema(
        min_length = 1,
        max_length = 128,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub name: String,
    #[schema(
        min_length = 1,
        max_length = 64,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub agent_version: String,
}

impl EnrollAgentRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !local::valid_credential(&self.node_credential) {
            return Err("node_credential must be 64 lowercase hexadecimal characters");
        }
        validate_text(&self.name, 128)?;
        validate_text(&self.agent_version, 64)
    }
}

/// Node identity is derived exclusively from the node bearer credential.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRequest {
    #[schema(
        min_length = 1,
        max_length = 64,
        pattern = r"[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]"
    )]
    pub agent_version: String,
}

impl HeartbeatRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_text(&self.agent_version, 64)
    }
}

/// Also lets a pending local enrollment recover its server-assigned node ID.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentNodeResponse {
    pub node: NodeSummary,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub message: String,
}

fn validate_text(value: &str, max: usize) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.chars().count() > max {
        return Err(
            "text must be within the documented character limit and contain non-whitespace text",
        );
    }
    Ok(())
}
