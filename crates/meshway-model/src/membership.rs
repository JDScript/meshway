use serde::Serialize;
use utoipa::ToSchema;

meshway_serde::string_enum! {
/// A user's role within one network; this does not grant instance privileges.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    /// Network manager able to grant all network roles, subject to last-Owner protection.
    Owner,
    /// Network manager able to grant the member role but not owner or admin.
    Admin,
    /// Network participant limited to authorized personal and related operational views.
    Member,
}

}
