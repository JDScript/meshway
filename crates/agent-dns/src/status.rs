//! Point-in-time service observation, independent of any IPC or HTTP envelope.

use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Active,
    Disabled,
    Expired,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub state: StatusState,
    pub generation: u64,
    pub revision_floor: u32,
    pub active_revision: Option<u32>,
    pub zone: Option<String>,
    pub ttl: Option<u32>,
    pub lease_remaining_ms: u64,
    pub listener: Option<SocketAddr>,
    pub listener_bound: bool,
    pub network_id: Option<String>,
    pub node_id: Option<String>,
}
