//! App Controller transport policy over protected socket capabilities.
mod carrier;
#[cfg(target_os = "linux")]
mod controller;
#[cfg(target_os = "linux")]
mod http;
pub(crate) use carrier::ManagedProtection;
#[cfg(target_os = "linux")]
mod bootstrap;
#[cfg(all(test, target_os = "linux"))]
mod tests;
#[cfg(target_os = "linux")]
pub(crate) use bootstrap::BootstrapChannel;
#[cfg(target_os = "linux")]
pub(crate) use controller::{ControllerChannel, ControllerClients};
#[cfg(target_os = "linux")]
fn closed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "underlay generation retired",
    )
}
#[cfg(target_os = "linux")]
fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
