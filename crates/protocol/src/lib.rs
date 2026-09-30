//! Wire DTOs used by the controller and its route-linked OpenAPI document.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Overview {
    pub name: String,
    pub version: String,
    pub node_count: u32,
}

/// A process report received by this development controller, not tunnel status.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub agent_version: String,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeList {
    pub nodes: Vec<NodeSummary>,
}

/// Unauthenticated development process reporting; this is not secure enrollment.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentReport {
    #[schema(min_length = 1, max_length = 128, pattern = r"\S")]
    pub node_id: String,
    #[schema(min_length = 1, max_length = 128, pattern = r"\S")]
    pub name: String,
    #[schema(min_length = 1, max_length = 64, pattern = r"\S")]
    pub agent_version: String,
}

impl AgentReport {
    /// Lengths use Unicode scalar values, matching JSON Schema string lengths.
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, max, message) in [
            (
                &self.node_id,
                128,
                "node_id must contain 1–128 characters and non-whitespace text",
            ),
            (
                &self.name,
                128,
                "name must contain 1–128 characters and non-whitespace text",
            ),
            (
                &self.agent_version,
                64,
                "agent_version must contain 1–64 characters and non-whitespace text",
            ),
        ] {
            if value.trim().is_empty() || value.chars().count() > max {
                return Err(message);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportResponse {
    pub node: NodeSummary,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub message: String,
}
