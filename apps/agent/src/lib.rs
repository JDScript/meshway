//! Agent-side HTTP interface generated from the controller's OpenAPI contract.

#[allow(dead_code, clippy::all)]
pub mod generated;

// Internal capability: never exported to IPC, Web or business `direct` routing.
#[cfg(feature = "managed-connections")]
#[allow(dead_code)] // Independently staged; runtime hooks are a separate handoff.
mod underlay;

#[cfg(feature = "managed-connections")]
mod endpoint_discovery;
pub mod netcheck;
#[cfg(feature = "managed-connections")]
pub use endpoint_discovery::{EndpointDiscoveryOptions, parse_stun_server};

// Share the production filesystem implementation with unit tests, while its
// fault hooks remain strictly cfg(test) and absent from production dependencies.
#[cfg(test)]
#[allow(dead_code)]
mod test_local;

#[cfg(test)]
#[path = "../tests/support/network_scope.rs"]
mod test_network_scope;

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
mod managed_dns;
#[cfg(feature = "managed-connections")]
pub use managed_dataplane::TunOptions;
#[cfg(feature = "managed-connections")]
pub use state::DnsService;

#[cfg(all(feature = "managed-connections", unix))]
pub mod diagnostics;
