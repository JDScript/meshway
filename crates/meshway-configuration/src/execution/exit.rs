use super::*;
use meshway_model::Digest;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Packet ownership only; grants no business Access. A Node computes it from
/// its own netmap and the rule-set artifacts it holds (`meshway-policy`); it
/// is never signed or distributed. Ranges are inclusive, ordered, disjoint
/// and non-adjacent, so each is one element of an nftables interval set.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressCapturePlan {
    /// Selection identity of the source egress plan it was computed for.
    pub generation: Digest,
    /// IPv4 ranges handed to the unique data-plane owner.
    pub managed_ipv4: Vec<(Ipv4Addr, Ipv4Addr)>,
    /// True for default IPv4 capture, including a first-match exit covering all IPv4.
    pub default_capture: bool,
    /// IPv6 exit ranges that must be sunk; Mesh routes take precedence.
    pub sink_ipv6: Vec<(Ipv6Addr, Ipv6Addr)>,
    /// Unresolved routing requires a fail-closed boundary, not local fallback.
    pub unavailable: Option<UnavailableReason>,
}

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Translation mode allowed for an exit role.
pub enum ExitNat {
    /// Stateful IPv4 source masquerade on the real egress interface.
    Masquerade,
}
}
