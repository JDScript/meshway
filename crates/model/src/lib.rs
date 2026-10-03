//! Shared value types for the Meshway control plane and nodes.
//!
//! Persistence records, credentials, configuration, and HTTP envelopes have
//! their own owners; they do not belong in this crate.

#![forbid(unsafe_code)]

mod digest;
mod ids;
mod membership;
mod numbers;

pub use digest::{Digest, InvalidDigest};
pub use ids::{
    ActivityId, ArtifactId, InstanceId, InvalidId, JoinLinkId, KeyId, MembershipId, NetworkId,
    NodeId, PublicationId, RequestId, SessionId, UserId,
};
pub use membership::MembershipRole;
pub use numbers::{InvalidNumber, MAX_SAFE_INTEGER, Revision, Sequence, Version};
