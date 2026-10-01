//! Agent-side HTTP interface generated from the controller's OpenAPI contract.

#[allow(dead_code, clippy::all)]
pub mod generated;

// Share the production filesystem implementation with unit tests, while its
// fault hooks remain strictly cfg(test) and absent from production dependencies.
#[cfg(test)]
#[allow(dead_code)]
mod test_local;

mod state;
pub use state::{
    Agent, AgentError, AgentStatus, ConnectionStatus, ConnectionSyncReceipt, ControlSyncReceipts,
    SyncReceipt, compiled_connection_capabilities, connections_status, status,
};

#[cfg(feature = "transport-lab")]
pub mod transport_lab;

#[cfg(feature = "managed-connections")]
mod managed_connections;

#[cfg(feature = "managed-connections")]
mod managed_dataplane;
#[cfg(feature = "managed-connections")]
pub use managed_dataplane::TunOptions;

#[cfg(all(feature = "managed-connections", unix))]
pub mod diagnostics;
