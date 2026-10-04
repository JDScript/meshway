//! Exact-trust TLS over already-open, caller-owned streams.
//!
//! Configurations are parsed and verified once by the constructors. Each
//! handshake owns its stream and drops it on failure, timeout or cancellation.
//! The configured handshake timeout is a local cap: callers must additionally
//! enclose the operation in their remaining overall connection/lease budget.
//! TLS authenticates the configured material, not a Meshway node identity or
//! permission. No sockets, DNS lookups, ambient roots or global crypto-provider
//! installation are used.
//!
//! Handshake timeouts must fit the platform's monotonic clock, including Tokio's
//! millisecond rounding, and may not exceed `i64::MAX` milliseconds (timer tick
//! headroom). Constructors reject invalid version ranges and malformed ALPN;
//! encoded ALPN lists are capped at 16 KiB.
#![forbid(unsafe_code)]

mod client;
mod error;
mod material;
mod options;
mod server;

pub use client::Client;
pub use error::Error;
pub use material::verify_listener;
pub use server::Server;
