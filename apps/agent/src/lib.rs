//! Agent-side HTTP interface generated from the controller's OpenAPI contract.

#[allow(dead_code, clippy::all)]
pub mod generated;

mod state;
pub use state::{Agent, AgentError, AgentStatus, status};
