//! Socket-scoped admission and exact namespace routing ownership.
//! No Controller HTTP operation, App identity file or CLI belongs here.
mod dns;
mod generation;
#[cfg(target_os = "linux")]
mod linux;
mod native;
#[cfg(target_os = "linux")]
mod owner;
mod socket;
#[cfg(all(test, target_os = "linux"))]
mod tests;
pub use dns::{BootstrapResolver, SystemDnsSnapshot};
pub use generation::PreparedGeneration;
#[cfg(target_os = "linux")]
pub use linux::{BootstrapUnderlay, LinuxUnderlay};
#[cfg(target_os = "linux")]
pub use owner::{BootstrapPreparation, CapturePrefix, Preparation, TunBinding};
pub use socket::{ProtectedListener, ProtectedTcp, ProtectedUdp, SocketFactory};
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Purpose {
    Controller,
    Carrier,
    AuthenticationProbe,
    Stun,
    BootstrapDns(BootstrapFor),
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BootstrapFor {
    Controller,
    Carrier,
    AuthenticationProbe,
    Stun,
}
impl Purpose {
    fn bootstrap(self) -> std::io::Result<Self> {
        Ok(Self::BootstrapDns(match self {
            Self::Controller => BootstrapFor::Controller,
            Self::Carrier => BootstrapFor::Carrier,
            Self::AuthenticationProbe => BootstrapFor::AuthenticationProbe,
            Self::Stun => BootstrapFor::Stun,
            Self::BootstrapDns(_) => {
                return Err(invalid("bootstrap DNS cannot recursively bootstrap"));
            }
        }))
    }
}
fn closed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "underlay generation retired",
    )
}
fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
