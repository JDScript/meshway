use std::{fmt, net::SocketAddr};

/// One opaque payload. For stream sessions `peer` must be absent or match the
/// session peer. Datagram sends require an explicit destination; receives carry
/// the actual source. No Mesh/IP/cryptographic header is interpreted here.
#[derive(Eq, PartialEq)]
pub struct Frame {
    pub payload: Vec<u8>,
    pub peer: Option<SocketAddr>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("payload_len", &self.payload.len())
            .field("payload_capacity", &self.payload.capacity())
            .field("peer", &self.peer)
            .finish()
    }
}

impl Frame {
    pub fn new(payload: Vec<u8>, peer: Option<SocketAddr>) -> Self {
        Self { payload, peer }
    }
}
