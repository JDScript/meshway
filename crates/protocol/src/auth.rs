use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId, UserId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{WireSecret, object::wire_object};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PasswordLengthUnit {
    /// Unicode scalar values, not UTF-16 code units, graphemes, or bytes.
    UnicodeCodepoints,
}

impl<'de> Deserialize<'de> for PasswordLengthUnit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A derived unit enum also accepts {"unicodeCodepoints": null}, which
        // disagrees with the published string schema.
        match String::deserialize(deserializer)?.as_str() {
            "unicodeCodepoints" => Ok(Self::UnicodeCodepoints),
            _ => Err(serde::de::Error::custom("unknown password length unit")),
        }
    }
}

wire_object! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PasswordRules {
        pub min_codepoints: u16 [minimum = 0, maximum = 65535],
        pub max_codepoints: u16 [minimum = 0, maximum = 65535],
        pub max_utf8_bytes: u16 [minimum = 0, maximum = 65535],
        pub length_unit: PasswordLengthUnit,
    }
}

wire_object! {
    #[derive(Debug)]
    pub struct AuthOptionsResponse {
        pub request_id: RequestId,
        pub registration_enabled: bool,
        pub password_rules: PasswordRules,
    }
}

wire_object! {
    pub struct RegisterRequest {
        pub email: String,
        pub password: WireSecret,
    }
}

wire_object! {
    pub struct LoginRequest {
        pub email: String,
        pub password: WireSecret,
    }
}

wire_object! {
    /// A user account; this view grants no network or instance privileges.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UserView {
        pub id: UserId,
        /// Canonical login identifier, not proof of email ownership.
        pub email: String,
    }
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SessionMetadata {
        pub id: SessionId,
        pub created_at: DateTime<Utc>,
        pub expires_at: DateTime<Utc>,
    }
}

wire_object! {
    /// First delivery only. Session reads never return the token.
    pub struct SessionIssuedResponse {
        pub request_id: RequestId,
        pub user: UserView,
        pub session: SessionMetadata,
        pub token: WireSecret,
    }
}

wire_object! {
    #[derive(Debug)]
    pub struct AuthSessionResponse {
        pub request_id: RequestId,
        pub user: UserView,
        pub session: SessionMetadata,
    }
}
