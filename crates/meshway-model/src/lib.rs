//! Shared value types for the Meshway control plane and nodes.
//!
//! Persistence records, credentials, configuration, and HTTP envelopes have
//! their own owners; they do not belong in this crate.

#![forbid(unsafe_code)]

mod digest;
pub mod exit;
mod ids;
mod membership;
pub mod netmap;
mod node;
pub use node::*;
pub mod networks;
mod numbers;
mod socket_address;
pub use networks::{MemberActions, MemberSummary, NetworkActions, NetworkSummary};
pub mod observations;
pub mod resources;
pub mod state;
pub mod timestamp;

pub use digest::{Digest, InvalidDigest};
pub use ids::{
    ActivityId, ArtifactId, ChallengeId, EnrollmentKeyId, EnrollmentRequestId, EventId, InstanceId,
    InvalidId, JoinLinkId, KeyId, MembershipId, NetworkId, NodeId, ObservationId,
    ObservationSubscriptionId, PublicationId, RequestId, SessionId, StateEpoch, UdpTraversalId,
    UserId,
};
pub use membership::MembershipRole;
pub use netmap::{NetmapLease, NetmapRef};
pub use numbers::{Count, InvalidNumber, MAX_SAFE_INTEGER, Revision, Sequence, Version};
pub use socket_address::{InvalidSocketAddress, SocketAddress};
