//! Execution authority handed from one issuer to Node components.
//!
//! The types here carry the lease deadline, invalidation and private material
//! of one verified netmap. They do not read storage, contact a Server or
//! decide policy. In production only `meshway-control` issues them; benchmark
//! and fixture issuers live under `tools/` and must never be assembled into
//! `meshwayd`. Holding or cloning a value never extends its grant.
#![forbid(unsafe_code)]

mod clock;
mod identity;
mod snapshot;

pub use clock::{Clock, Deadline, Started};
pub use identity::{DataPlaneIdentity, packet_proof_message};
pub use snapshot::{ExecutionSnapshot, NetmapIssue, NetmapReading, NetmapRenewal};


#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the execution grant has expired or was invalidated")]
    Expired,
    #[error("issued material does not match the execution target")]
    Contract,
}

/// Wall-clock milliseconds. Callers combine it with a `Clock` so that a
/// backwards adjustment cannot revive an expired grant.
pub fn now() -> Result<u64, Error> {
    chrono::Utc::now()
        .timestamp_millis()
        .try_into()
        .map_err(|_| Error::Expired)
}
