//! Exact-trust TLS over already-open, caller-owned streams.
//!
//! Configurations are parsed and verified once by the constructors. Each
//! handshake owns its stream and drops it on failure, timeout or cancellation.
//! The configured handshake timeout is a local cap: callers must additionally
//! enclose the operation in their remaining overall connection/lease budget.
//! TLS authenticates the configured material, not a Meshway node identity or
//! permission. Managed trust never uses ambient roots. The optional
//! `platform-verifier` feature provides the system-trust HTTP configuration.
//! Neither path opens sockets, resolves DNS or installs a global crypto provider.
//!
//! Handshake timeouts must fit the platform's monotonic clock, including Tokio's
//! millisecond rounding, and may not exceed `i64::MAX` milliseconds (timer tick
//! headroom). Constructors reject invalid version ranges and malformed ALPN;
//! encoded ALPN lists are capped at 16 KiB.
mod client;
mod error;
mod material;
mod options;
#[cfg(feature = "platform-verifier")]
mod platform;
mod server;

pub use client::Client;
pub use error::Error;
pub use material::verify_listener;
#[cfg(feature = "platform-verifier")]
pub use platform::platform_client_config;
pub use server::Server;
