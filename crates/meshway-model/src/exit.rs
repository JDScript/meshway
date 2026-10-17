//! Exit observations are facts, never forwarding authority.
use crate::{Digest, InstanceId, NetmapRef};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// Acquire capability key for the Linux IPv4 masquerade executor.
pub const EXIT_IPV4_FEATURE: &str = "feature:exit-ipv4";
/// Version of that backend contract, independent of the encrypted packet wire.
pub const EXIT_IPV4_VERSION: u16 = 1;
/// Acquire capability key for the Linux IPv6 masquerade (NAT66) executor.
pub const EXIT_IPV6_FEATURE: &str = "feature:exit-ipv6";
/// Version of that backend contract.
pub const EXIT_IPV6_VERSION: u16 = 1;

crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Inner address family, independent of the outer carrier.
pub enum ExitFamily {
    /// IPv4 inner packets.
    Ipv4,
    /// IPv6 inner packets.
    Ipv6,
}
}

impl ExitFamily {
    /// The family of an inner address.
    pub fn of(address: std::net::IpAddr) -> Self {
        match address.to_canonical() {
            std::net::IpAddr::V4(_) => Self::Ipv4,
            std::net::IpAddr::V6(_) => Self::Ipv6,
        }
    }

    /// Acquire capability key and version of this family's executor.
    pub fn feature(self) -> (&'static str, u16) {
        match self {
            Self::Ipv4 => (EXIT_IPV4_FEATURE, EXIT_IPV4_VERSION),
            Self::Ipv6 => (EXIT_IPV6_FEATURE, EXIT_IPV6_VERSION),
        }
    }
}
crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Concrete prerequisite preventing exit installation or execution.
pub enum ExitUnavailableReason {
    /// Host IPv4 forwarding is disabled.
    IpForwardDisabled,
    /// Host IPv6 forwarding (`net.ipv6.conf.all.forwarding`) is disabled.
    Ipv6ForwardDisabled,
    /// The host has no global IPv6 address or no IPv6 route toward the Internet.
    Ipv6NoEgress,
    /// The IPv6 egress route comes from router advertisements that forwarding makes the host ignore (`accept_ra` is not 2).
    Ipv6RouterAdvertisementsIgnored,
    /// Host forwarding could not be read or verified.
    ForwardingProbeFailed,
    /// A real NET_ADMIN operation failed.
    NetAdminMissing,
    /// Required nftables operations are unavailable.
    NftUnavailable,
    /// Owned conntrack management is unavailable.
    ConntrackUnavailable,
    /// Reverse path filtering conflicts with the owned routing plan.
    ReversePathFilterConflict,
    /// Required TUN operations are unavailable.
    TunUnavailable,
    /// Host firewall prevents forwarding.
    FirewallConflict,
    /// Host routes conflict with the owned plan.
    RouteConflict,
    /// Requested inner address family is unsupported.
    UnsupportedFamily,
    /// Ownership of required resources cannot be proved.
    OwnershipConflict,
}
}
crate::tagged! { tag = "status";
#[derive(Clone, Debug, PartialEq, Eq)]
/// Observed exit lifecycle; unavailable carries a mandatory typed cause.
pub enum ExitStatus {
    /// No exit role has been requested.
    NotRequested,
    /// Prerequisites are being tested; not ready.
    Probing,
    /// The reported families completed prerequisite checks.
    Ready,
    /// Installation or prerequisite checks failed.
    Unavailable {
        /// Concrete failure; never replaced with a successful observation.
        reason: ExitUnavailableReason,
    },
}
}
crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Why an exit resolver cannot currently resolve its sources' queries with the exit host's OS resolver.
pub enum ExitDnsUnavailableReason {
    /// The OS resolver could not be observed, for example before the owned DNS link is installed.
    Unobserved,
    /// The OS resolver lists no server.
    NoServers,
    /// An OS server is a loopback, unspecified, multicast or otherwise unusable local address, which may be a stub that recurses into Meshway.
    LocalServer,
    /// An OS server is this Node's own DNS anchor address.
    AnchorServer,
    /// An OS server is inside a Mesh address pool.
    MeshServer,
    /// This Node's egress partition does not leave an OS server to a local socket (Direct or Unmanaged).
    ExitRoutedServer,
    /// systemd-resolved's proxy stub 127.0.0.54:53 (systemd ≥ 248 with `DNSStubListener=yes`) is not listening.
    ProxyStubMissing,
}
}
crate::tagged! { tag = "status";
#[derive(Clone, Debug, PartialEq, Eq)]
/// Whether the exit resolver can currently serve its sources.
pub enum ExitDnsStatus {
    /// Sources' queries are resolved by the exit host's OS resolver, or by the configured default resolvers under `dns.overrideLocal`.
    Serving,
    /// Sources' queries are answered SERVFAIL; the Node's own DNS is unaffected.
    Unavailable {
        /// Concrete cause.
        reason: ExitDnsUnavailableReason,
    },
}
}
crate::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Exit resolver fact of the installed netmap's `serveExitResolver` plan.
pub struct ExitDns {
    /// Exact installed netmap whose served plan this describes.
    pub netmap: NetmapRef,
    /// Current availability for the plan's sources.
    pub status: ExitDnsStatus,
}
}
crate::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// One inner family of an exit role: its own prerequisites and installation.
pub struct ExitFamilyCapability {
    /// Inner family this entry describes.
    pub family: ExitFamily,
    /// Prerequisite result of this family; shared prerequisites fail both families with the same reason.
    pub status: ExitStatus,
    /// Whether this family's forwarding resources are installed; ready alone does not imply installed.
    pub installed: bool,
} }
crate::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Target-bound exit capability observation, accepted before applied as well as after it.
pub struct ExitCapability {
    /// Runtime incarnation which made the observation.
    pub instance_id: InstanceId,
    /// Exact netmap whose role was probed.
    pub netmap: NetmapRef,
    /// Generation of that netmap's exit role.
    pub role_generation: Digest,
    /// One entry per family of the role, ascending and unique.
    pub families: Vec<ExitFamilyCapability>,
    /// Original observation time, preserved on retries.
    pub observed_at: DateTime<Utc>,
}
}

impl ExitCapability {
    /// The entry of `family`, if the role has it.
    pub fn family(&self, family: ExitFamily) -> Option<&ExitFamilyCapability> {
        self.families.iter().find(|entry| entry.family == family)
    }

    /// Whether `family` is ready and installed: a new exit flow of it may
    /// choose this exit.
    pub fn serves(&self, family: ExitFamily) -> bool {
        self.family(family)
            .is_some_and(|entry| entry.status == ExitStatus::Ready && entry.installed)
    }

    /// Whether the role may install: its IPv4 family is ready. A role
    /// always carries IPv4; its IPv6 family installs with it and is
    /// selectable while its own prerequisites hold.
    pub fn installable(&self) -> bool {
        self.family(ExitFamily::Ipv4)
            .is_some_and(|entry| entry.status == ExitStatus::Ready)
    }

    /// Whether any family is installed.
    pub fn installed(&self) -> bool {
        self.families.iter().any(|entry| entry.installed)
    }

    /// Report every family with `status` (a shared condition).
    pub fn set_status(&mut self, status: ExitStatus) {
        for entry in &mut self.families {
            entry.status = status.clone();
        }
    }

    /// Mark every family installed or not, as the role is.
    pub fn set_installed(&mut self, installed: bool) {
        for entry in &mut self.families {
            entry.installed = installed;
        }
    }
}
