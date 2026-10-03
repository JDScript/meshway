use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

/// A user's role within one network; this does not grant instance privileges.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    Owner,
    Admin,
    Member,
}

impl<'de> Deserialize<'de> for MembershipRole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            _ => Err(serde::de::Error::custom("unknown membership role")),
        }
    }
}
