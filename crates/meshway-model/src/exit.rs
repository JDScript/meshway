//! Exit observations are facts, never forwarding authority.
use crate::{Digest, InstanceId, NetmapRef};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// Acquire capability key for the initial Linux IPv4 masquerade executor.
pub const EXIT_IPV4_FEATURE: &str = "feature:exit-ipv4";
/// Version of that backend contract, independent of the encrypted packet wire.
pub const EXIT_IPV4_VERSION: u16 = 1;

crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Inner address family, independent of the outer carrier.
pub enum ExitFamily {
    /// IPv4 inner packets.
    Ipv4,
    /// IPv6 inner packets; unsupported by the initial exit executor.
    Ipv6,
}
}
crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Concrete prerequisite preventing exit installation or execution.
pub enum ExitUnavailableReason {
    /// Host IPv4 forwarding is disabled.
    IpForwardDisabled,
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
    /// Required resolver execution is unavailable.
    DnsUnavailable,
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
/// Target-bound exit capability observation, accepted before applied as well as after it.
pub struct ExitCapability {
    /// Runtime incarnation which made the observation.
    pub instance_id: InstanceId,
    /// Exact netmap whose role was probed.
    pub netmap: NetmapRef,
    /// Generation of that netmap's exit role.
    pub role_generation: Digest,
    /// Families supported by the running backend.
    pub supported_families: Vec<ExitFamily>,
    /// Families actually installed; ready alone does not imply installed.
    pub installed_families: Vec<ExitFamily>,
    /// Actual prerequisite or installation result.
    pub status: ExitStatus,
    /// Original observation time, preserved on retries.
    pub observed_at: DateTime<Utc>,
}
}
