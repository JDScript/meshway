//! Test-only Server boundary for Node crates.
//!
//! Node crate tests depend on this crate alone for a Server: it starts the
//! real Server in-process ([`TestServer`]), enrolls production `Control`s,
//! and re-exports the generated [`client`].
//!
//! Fixtures that compile their own netmaps cannot obtain the real Server's
//! sealed signing key. They enroll their production `Control` through
//! [`PinningProxy`], which forwards every request to the real Server and only
//! replaces the `serverSigningKey` pinned by the enrollment response with the
//! fixed [`server_key`]. Scripted sync replies are then signed with that key
//! ([`signed`]), so the Control verifies them exactly as it verifies the real
//! Server. Nothing here is reachable from production code.
pub mod configuration;
mod proxy;
mod server;
mod sync;

pub use configuration::single_node;
pub use meshway_control_client as client;
pub use proxy::PinningProxy;
pub use server::{Options, Owner, RESPONSE_LIMIT, TestServer, http, random_id, register};
pub use sync::{
    Facts, Reply, Script, lease, now_ms, reply, server_key, sign, sign_associations, sign_with,
    signed,
};
