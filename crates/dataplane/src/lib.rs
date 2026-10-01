#![forbid(unsafe_code)]

mod config;
mod crypto;
mod framing;
pub mod packet_io;
mod runtime;

pub use config::{
    LabConfig, Limits, ListenerConfig, OutgoingCandidate, PeerConfig, PrivateKey, TransportKind,
};
pub use crypto::Identity;
pub use framing::{
    Batch, BoxedStream, DuplexStream, MAX_BATCH_BYTES, MAX_BATCH_FRAMES, MAX_FRAME_SIZE,
};
pub use runtime::{
    BoundListener, DropStatistics, MAX_NATIVE_UDP_DATAGRAM, MAX_PACKET_PAYLOAD,
    NeighborProbeObservation, PeerConnection, ProbeReply, QueueRejectionStatistics,
    QueueStatistics, ReceivedPacket, Runtime, RuntimeQueueStatistics, Statistics, StreamContext,
    StreamRole, TransportStatistics, UdpDiscoveryObservation, UdpDiscoveryOutcome, UdpSocketInfo,
    native_udp_datagram_len,
};

/// Intentionally contains no cryptographic material, endpoints or application bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfig(&'static str),
    Authentication,
    Protocol,
    Oversize,
    Replay,
    UnauthorizedPath,
    UnauthorizedTransport,
    Unavailable,
    Capacity,
    Timeout,
    Closed,
    Io,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(f, "invalid lab configuration: {reason}"),
            other => write!(f, "dataplane {other:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

pub type Result<T> = std::result::Result<T, Error>;
