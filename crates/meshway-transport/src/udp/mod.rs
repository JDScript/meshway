//! Native UDP endpoints with explicit source and truncation metadata.
//! The host opens/protects the socket and decides permitted peers.
pub use crate::raw::Buffers;
mod drops;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use drops::inode;
pub use drops::receive_drops;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod endpoint;
#[cfg(target_os = "linux")]
pub use endpoint::connected_path_mtu;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use endpoint::{Capabilities, DEFAULT_RECEIVE_BUFFER, DEFAULT_SEND_BUFFER, UdpTransport};
