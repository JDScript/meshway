//! This Node's registered packet-plane keys and the one hop-proof transcript.
//!
//! Keys are not authority: a consumer uses them only for desired netmap
//! content whose registered public keys they match. Nothing here carries a
//! lease, deadline or Server state; Engine owns the netmap lease.
#![forbid(unsafe_code)]

mod identity;

pub use identity::{DataPlaneKeys, packet_proof_message};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the key does not match this Node's registered packet key")]
    Contract,
}
