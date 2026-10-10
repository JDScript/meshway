use super::*;
use meshway_model::Digest;

object! {
/// Packet ownership only. These disjoint CIDRs grant no business Access.
pub struct EgressCapturePlan {
    /// Identity of the effective selection and its routing inputs.
    pub generation: Digest,
    /// Disjoint IPv4 ranges handed to the unique data-plane owner.
    pub managed_ipv4: Vec<String>,
    /// True for default IPv4 capture, including a first-match exit covering all IPv4.
    pub default_capture: bool,
    /// Explicit direct IPv4 exceptions after first-match subtraction.
    pub direct_ipv4: Vec<String>,
    /// Explicit unmanaged IPv4 exceptions after first-match subtraction.
    pub unmanaged_ipv4: Vec<String>,
    /// IPv6 exit ranges that must be sunk; Mesh routes take precedence.
    pub sink_ipv6: Vec<String>,
    /// Unresolved routing requires a fail-closed boundary, not local fallback.
    pub unavailable: Option<UnavailableReason>,
} }
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Translation mode allowed for an exit role.
pub enum ExitNat {
    /// Stateful IPv4 source masquerade on the real egress interface.
    Masquerade,
}
}
