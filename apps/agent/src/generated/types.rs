//! Generated types from OpenAPI specification
//!
//! This file contains all the generated types for the API.
//! Do not edit manually - regenerate using the appropriate script.
#![allow(clippy::large_enum_variant)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::let_unit_value)]
#![allow(unreachable_patterns)]
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReportResponse {
    pub node: NodeSummary,
}
///A process report received by this development controller, not tunnel status.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeSummary {
    pub agent_version: String,
    pub id: String,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub name: String,
}
///Unauthenticated development process reporting; this is not secure enrollment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentReport {
    ///Constraint: minLength=1, maxLength=64, pattern=`\S`
    pub agent_version: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`\S`
    pub name: String,
    ///Constraint: minLength=1, maxLength=128, pattern=`\S`
    pub node_id: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub message: String,
}
