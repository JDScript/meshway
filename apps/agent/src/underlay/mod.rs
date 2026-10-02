//! Internal outer-socket capability. Does not grant business access or own sinks.
mod controller;
mod dns;
#[cfg(all(test, target_os = "linux"))]
#[path = "tests/support/sockets.rs"]
mod fixture;
mod generation;
mod http;
mod managed;
#[cfg(target_os = "linux")]
mod owner;
#[allow(unused_imports)]
pub(crate) use managed::ManagedProtection;
#[cfg(target_os = "linux")]
pub(crate) use owner::{BootstrapChannel, BootstrapPreparation, BootstrapUnderlay};
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub(crate) use owner::{CapturePrefix, LinuxUnderlay, Preparation, TunBinding};
mod socket;

#[allow(unused_imports)]
pub(crate) use controller::{ControllerChannel, ControllerClients};
#[allow(unused_imports)] // Routing owner and carrier bootstrap adapter.
pub(crate) use dns::{BootstrapResolver, SystemDnsSnapshot};
#[allow(unused_imports)] // Staged owner integration is in the handoff.
pub(crate) use generation::{MarkReservation, PreparedGeneration, Reservation};
#[allow(unused_imports)] // Carrier integration names these wrapper types.
pub(crate) use socket::{ProtectedListener, ProtectedTcp, ProtectedUdp, SocketFactory};

/// Never deserialize from an external request or routing rule.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Purpose {
    Controller,
    Carrier,
    AuthenticationProbe,
    Stun,
    BootstrapDns(BootstrapFor),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BootstrapFor {
    Controller,
    Carrier,
    AuthenticationProbe,
    Stun,
}

impl Purpose {
    fn bootstrap(self) -> std::io::Result<Self> {
        let purpose = match self {
            Self::Controller => BootstrapFor::Controller,
            Self::Carrier => BootstrapFor::Carrier,
            Self::AuthenticationProbe => BootstrapFor::AuthenticationProbe,
            Self::Stun => BootstrapFor::Stun,
            Self::BootstrapDns(_) => {
                return Err(invalid("bootstrap DNS cannot recursively bootstrap"));
            }
        };
        Ok(Self::BootstrapDns(purpose))
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
