use chrono::{DateTime, Utc};
use meshway_model::RequestId;
pub use meshway_model::{MemberActions, MemberSummary, NetworkActions, NetworkSummary};

use crate::object::wire_object;
use meshway_model::wire::required_nullable;

/// URI query fields, not a JSON-body DTO. Omission differs from an empty cursor.
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct NetworkPageQuery {
    /// Maximum number of items to return, from 1 through 200; omission uses the operation default.
    #[serde(default, deserialize_with = "optional_limit")]
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u16>,
    /// Opaque continuation cursor from the preceding page, limited to 4096 characters.
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
    /// Request to create an isolated network and an Owner membership for the caller.
    pub struct CreateNetworkRequest {
        /// Caller-chosen idempotency key bound to the authenticated user and creation input.
        pub request_id: RequestId,
        /// Raw input. Apply getAuthOptions.networkNameRules; padding is allowed.
        pub name: String,
    }
}

wire_object! {
    #[derive(Debug)]
    /// Network rename guarded by the previously read name and a request idempotency key.
    pub struct RenameNetworkRequest {
        /// Caller-chosen idempotency key bound to the caller, network, and rename input.
        pub request_id: RequestId,
        /// Raw input. Apply getAuthOptions.networkNameRules, not String.length.
        pub name: String,
        /// Previously read network name that must still match when the rename is committed.
        pub expected_name: String,
    }
}

wire_object! {
    #[derive(Debug)]
    /// A network projected for the caller's current membership.
    pub struct NetworkResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Network metadata and the caller's current role and action hints.
        pub network: NetworkSummary,
    }
}

wire_object! {
    #[derive(Debug)]
    /// A bounded page of networks in which the caller currently has membership.
    pub struct NetworkListResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Visible networks in ascending creation order within the initial query bounds.
        pub items: Vec<NetworkSummary>,
        /// Opaque continuation cursor, or null when no further page is available.
        #[serde(deserialize_with = "required_nullable")]
        pub next_cursor: Option<String> [required = true],
        /// Time at which the initial pagination window was established.
        pub as_of: DateTime<Utc>,
        /// Current visible count within the cursor's original creation bounds.
        /// Recomputed on every page; permission loss can reduce this value.
        pub total: meshway_model::Count,
    }
}

wire_object! {
    #[derive(Debug)]
    /// A bounded page of current network members for an authorized manager.
    pub struct NetworkMembersResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Current members in ascending membership creation order.
        pub items: Vec<MemberSummary>,
        /// Opaque continuation cursor, or null when no further page is available.
        #[serde(deserialize_with = "required_nullable")]
        pub next_cursor: Option<String> [required = true],
        /// Time at which the initial pagination window was established.
        pub as_of: DateTime<Utc>,
        /// Current visible count within the cursor's original creation bound.
        /// Removed members disappear and reduce total on subsequent pages.
        pub total:meshway_model::Count,
    }
}
