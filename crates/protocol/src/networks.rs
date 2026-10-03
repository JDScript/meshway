use chrono::{DateTime, Utc};
use meshway_model::{
    MembershipId, MembershipRole, NetworkId, RequestId, Revision, UserId, Version,
};

use crate::object::wire_object;

/// URI query fields, not a JSON-body DTO. Omission differs from an empty cursor.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct NetworkPageQuery {
    #[serde(default, deserialize_with = "optional_limit")]
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u16>,
    #[serde(default, deserialize_with = "optional_cursor")]
    #[param(max_length = 4096)]
    pub cursor: Option<String>,
}

fn optional_limit<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u16>, D::Error> {
    let value = <Option<u16> as serde::Deserialize>::deserialize(deserializer)?;
    if value.is_some_and(|limit| !(1..=200).contains(&limit)) {
        return Err(serde::de::Error::custom("page limit is out of range"));
    }
    Ok(value)
}

fn optional_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = <Option<String> as serde::Deserialize>::deserialize(deserializer)?;
    if value
        .as_ref()
        .is_some_and(|cursor| cursor.chars().count() > 4096)
    {
        return Err(serde::de::Error::custom("pagination cursor is too long"));
    }
    Ok(value)
}

wire_object! {
    #[derive(Debug)]
    pub struct CreateNetworkRequest {
        pub request_id: RequestId,
        pub name: String,
    }
}

wire_object! {
    #[derive(Debug)]
    pub struct RenameNetworkRequest {
        pub request_id: RequestId,
        pub name: String,
        pub expected_name: String,
    }
}

wire_object! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NetworkActions {
        pub rename: bool,
        pub edit_configuration: bool,
        pub manage_members: bool,
    }
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
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

wire_object! {
    #[derive(Debug)]
    pub struct NetworkResponse {
        pub request_id: RequestId,
        pub network: NetworkSummary,
    }
}

wire_object! {
    #[derive(Debug)]
    pub struct NetworkListResponse {
        pub request_id: RequestId,
        pub items: Vec<NetworkSummary>,
        #[serde(deserialize_with = "required_nullable")]
        pub next_cursor: Option<String> [required = true],
        pub as_of: DateTime<Utc>,
    }
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MemberSummary {
        pub membership_id: MembershipId,
        pub user_id: UserId,
        pub email: String,
        pub role: MembershipRole,
        pub version: Version,
        pub joined_at: DateTime<Utc>,
    }
}

wire_object! {
    #[derive(Debug)]
    pub struct NetworkMembersResponse {
        pub request_id: RequestId,
        pub items: Vec<MemberSummary>,
        #[serde(deserialize_with = "required_nullable")]
        pub next_cursor: Option<String> [required = true],
        pub as_of: DateTime<Utc>,
    }
}

fn required_nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    serde::Deserialize::deserialize(deserializer)
}
