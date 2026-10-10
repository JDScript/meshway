//! Native UDP endpoints with explicit source and truncation metadata.
//! The host opens/protects the socket and decides permitted peers.
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod endpoint;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use endpoint::{Capabilities, UdpTransport};
