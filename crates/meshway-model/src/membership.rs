use serde::Serialize;
use utoipa::ToSchema;

meshway_serde::string_enum! {
/// A user's role within one network; this does not grant instance privileges.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    Owner,
    Admin,
    Member,
}

}
