//! Stream carriers over caller-owned byte streams: exact-trust TLS, and the
//! VLESS and Trojan TCP CONNECT encapsulations built on it.
//!
//! None of these opens sockets, resolves DNS, authenticates a Meshway Node or
//! authorizes routing; the host supplies streams and decides destinations.
#![forbid(unsafe_code)]

pub mod tls;
pub mod trojan;
pub mod vless;
