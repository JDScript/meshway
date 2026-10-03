use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A user's role within one network; this does not grant instance privileges.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    Owner,
    Admin,
    Member,
}
