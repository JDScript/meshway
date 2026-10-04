//! SQL behind configuration versions, snapshots, idempotent commits, listener
//! materials and the trusted inputs a normalization reads.

pub(crate) mod addresses;
pub(crate) mod commits;
pub(crate) mod context;
pub(crate) mod materials;
pub(crate) mod memberships;
pub(crate) mod resources;
pub(crate) mod versions;
