//! Shared public network/member fragments, independent of HTTP envelopes.
use crate::{MembershipId, MembershipRole, NetworkId, Revision, UserId, Version};
use chrono::{DateTime, Utc};

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current network operation hints; mutations independently recheck authorization.
pub struct NetworkActions {
    /// Whether the caller may rename this network.
    pub rename: bool,
    /// Whether `GET /configuration` (the complete, management view) is
    /// readable; otherwise the member view of own devices applies.
    pub read_configuration: bool,
    /// Whether the caller may edit the complete network configuration.
    pub edit_configuration: bool,
    /// Whether the caller may read and manage network members, subject to target-role constraints.
    pub manage_members: bool,
    /// Roles the caller may grant through join links, ordered owner, admin, then member.
    pub join_link_roles: Vec<MembershipRole>,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Network metadata and the requesting user's current membership and permissions.
pub struct NetworkSummary {
    /// Stable identifier of the network.
    pub id: NetworkId,
    /// Validated network name after whitespace normalization.
    pub name: String,
    /// Time at which the network was created.
    pub created_at: DateTime<Utc>,
    /// Current saved configuration revision, independent of Node application status.
    pub current_revision: Revision,
    /// Requesting user's current membership in this network.
    pub membership_id: MembershipId,
    /// Requesting user's current network role.
    pub role: MembershipRole,
    /// Operations currently available to the requesting user.
    pub actions: NetworkActions,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Target-specific membership operation hints, including last-Owner protection.
pub struct MemberActions {
    /// Whether the caller may assign a role different from the member's current role.
    pub change_role: bool,
    /// Roles currently assignable to this member, ordered owner, admin, then member.
    pub assignable_roles: Vec<MembershipRole>,
    /// Whether the caller may remove this member or leave their own membership.
    pub remove: bool,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Current network membership with caller-specific operation hints.
pub struct MemberSummary {
    /// Stable identifier of this membership, not reused when a removed user rejoins.
    pub membership_id: MembershipId,
    /// Stable account identifier of the member.
    pub user_id: UserId,
    /// Member's current canonical login address.
    pub email: String,
    /// Member's current role within this network.
    pub role: MembershipRole,
    /// Concurrency version required by membership mutations.
    pub version: Version,
    /// Time at which this membership was created.
    pub joined_at: DateTime<Utc>,
    /// Operations the caller may currently perform on this member.
    pub actions: MemberActions,
}
}
