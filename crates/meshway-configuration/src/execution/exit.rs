use super::*;
use meshway_model::{InstanceId, exit::ExitFamily};

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
object! {
/// Resource preparation responsibility; never a grant to forward a packet.
/// Network, node, publication and deadlines come from the enclosing target/lease.
pub struct ExitRolePlan {
    /// Current instance which may install the role.
    pub instance_id: InstanceId,
    /// Role identity stable across lease renewal; changes with source revision.
    pub generation: Digest,
    /// Inner family implemented by this role.
    pub family: ExitFamily,
    /// Required translation mode.
    pub nat: ExitNat,
} }
tagged! {
/// Why an exact external destination is eligible; never widens PathPermit.target.
pub enum ExitDestinationScope {
    /// Exact target must also belong to Policy's public Internet set.
    Internet,
    /// Explicit CIDR Access covers the exact target, including authorized LAN.
    Cidr {
        /// Administrator-authorized CIDR, not an automatically inferred LAN.
        prefix: String
    },
} }
object! {
/// Additional constraints on an external PathPermit; paths name its exit terminals.
pub struct ExitPermit {
    /// Changes on every effective profile selection, including switching back.
    pub generation: Digest,
    /// Source address family; initial implementation permits IPv4 only.
    pub family: ExitFamily,
    /// Destination classification checked again by packet admission.
    pub destination: ExitDestinationScope,
    /// Every eligible terminal bound to the role generation it must install.
    pub roles: BTreeMap<NodeId, ExitRolePlan>,
} }
