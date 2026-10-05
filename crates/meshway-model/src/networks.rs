//! Shared public network/member fragments, independent of HTTP envelopes.
use crate::{MembershipId, MembershipRole, NetworkId, Revision, UserId, Version};
use chrono::{DateTime, Utc};

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkActions {
    pub rename: bool,
    /// Whether `GET /configuration` (the complete, management view) is
    /// readable; otherwise the member view of own devices applies.
    pub read_configuration: bool,
    pub edit_configuration: bool,
    pub manage_members: bool,
    pub join_link_roles: Vec<MembershipRole>,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSummary {
    pub id: NetworkId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub current_revision: Revision,
    pub membership_id: MembershipId,
    pub role: MembershipRole,
    pub actions: NetworkActions,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberActions {
    pub change_role: bool,
    pub assignable_roles: Vec<MembershipRole>,
    pub remove: bool,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberSummary {
    pub membership_id: MembershipId,
    pub user_id: UserId,
    pub email: String,
    pub role: MembershipRole,
    pub version: Version,
    pub joined_at: DateTime<Utc>,
    pub actions: MemberActions,
}
}
