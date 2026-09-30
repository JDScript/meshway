//! Generated types from OpenAPI specification
//!
//! This file contains all the generated types for the API.
//! Do not edit manually - regenerate using the appropriate script.
#![allow(clippy::large_enum_variant)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::let_unit_value)]
#![allow(unreachable_patterns)]
use serde::{Deserialize, Serialize};
///Also lets a pending local enrollment recover its server-assigned node ID.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentNodeResponse {
    pub node: NodeSummary,
}
///Durable control-plane identity. Last contact is not tunnel health.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeSummary {
    pub agent_version: String,
    pub id: String,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub name: String,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}
/**Persist the proposed random credential before sending this request. Retry
with the same enrollment token, credential, name and enrollment version.*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrollAgentRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub name: String,
    ///Constraint: minLength=64, maxLength=64, pattern=`^[0-9a-f]{64}$`
    pub node_credential: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub message: String,
}
///Node identity is derived exclusively from the node bearer credential.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeartbeatRequest {
    ///Constraint: minLength=1, maxLength=64, pattern=`[^\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`
    pub agent_version: String,
}

// Generated from required nullable OpenAPI properties. deserialize_with makes
// serde require presence; Option accepts explicit null. Never skip None on output.
fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
