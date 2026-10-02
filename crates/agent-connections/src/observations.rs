//! Connection-domain observations. No IPC schema, serialization or CLI messages.
use meshway_protocol::ConnectionTransport;
use std::time::Duration;

pub const MAX_PEERS: u32 = 64;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Unavailable,
    UnconfiguredPeer,
    AmbiguousPeer,
    ChannelUnavailable,
    AuthenticationFailed,
    Timeout,
    Busy,
    InvalidRequest,
    UnknownIdentity,
}
#[derive(Clone)]
pub struct ProbeRequest {
    pub peer: String,
    pub timeout: Duration,
}
impl ProbeRequest {
    pub fn validate(&self) -> Result<(), ProbeError> {
        if self.peer.trim().is_empty()
            || self.peer.chars().count() > 128
            || !(Duration::from_secs(1)..=Duration::from_secs(10)).contains(&self.timeout)
        {
            return Err(ProbeError::InvalidRequest);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicIdentity {
    pub node_id: String,
    pub name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}
#[derive(Debug)]
pub struct AuthenticatedChannel {
    pub peer_node_id: String,
    pub initiator_node_id: String,
    pub transport: ConnectionTransport,
}
#[derive(Debug)]
pub struct LiveStatus {
    pub generation: u64,
    pub installed_revision: u32,
    pub source_revision: u32,
    pub lease_remaining: Duration,
    pub local_node_id: String,
    pub local_identity: Option<PublicIdentity>,
    pub authenticated_channel_count: u32,
    pub peers: Option<Vec<AuthenticatedChannel>>,
    pub peer_offset: u32,
    pub next_peer_offset: Option<u32>,
}
#[derive(Debug)]
pub struct WhoisObservation {
    pub generation: u64,
    pub installed_revision: u32,
    pub source_revision: u32,
    pub identity: PublicIdentity,
}
#[derive(Debug)]
pub struct ProbeObservation {
    pub generation: u64,
    pub installed_revision: u32,
    pub source_revision: u32,
    pub peer_node_id: String,
    pub peer_name: Option<String>,
    pub local_node_id: String,
    pub initiator_node_id: String,
    pub transport: ConnectionTransport,
    pub round_trip: Duration,
}
