//! Agent-side HTTP interface generated from the controller's OpenAPI contract.

#[allow(dead_code, clippy::all)]
pub mod generated;

// Share the production filesystem implementation with unit tests, while its
// fault hooks remain strictly cfg(test) and absent from production dependencies.
#[cfg(test)]
#[allow(dead_code)]
mod test_local;

mod state;
pub use state::{Agent, AgentError, AgentStatus, SyncReceipt, status};

#[cfg(feature = "transport-lab")]
pub mod transport_lab;
