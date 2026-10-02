//! Adjacency/carrier execution. No TUN, packet plan, HTTP client or CLI output.
mod configuration;
pub mod observations;
pub mod profiles;
pub mod session;
pub mod sockets;
pub use configuration::{controller_candidate, core_config};
pub use meshway_protocol::{ConnectionRuntimeReason as Reason, ConnectionRuntimeState as Phase};

#[derive(Clone, Copy, Debug)]
pub struct Failure(pub Reason);
#[derive(Clone, serde::Serialize)]
pub struct Observation {
    pub state: Phase,
    pub reason: Reason,
    pub listeners: Vec<meshway_protocol::ConnectionListenerObservation>,
    pub peers: Vec<meshway_protocol::ConnectionNeighborObservation>,
}
impl Observation {
    pub fn empty(state: Phase, reason: Reason) -> Self {
        Self {
            state,
            reason,
            listeners: Vec::new(),
            peers: Vec::new(),
        }
    }
}
use configuration::{allowed_peers, candidate_label, protocol_transport};
