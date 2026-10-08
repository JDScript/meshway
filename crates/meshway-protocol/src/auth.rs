use chrono::{DateTime, Utc};
use meshway_model::{RequestId, SessionId, UserId};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{WireSecret, object::wire_object};
use meshway_serde::required_nullable as nullable;

meshway_serde::string_enum! {
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Unit used to measure password length before UTF-8 byte limits are applied.
pub enum PasswordLengthUnit {
    /// Unicode scalar values, not UTF-16 code units, graphemes, or bytes.
    UnicodeCodepoints,
}

}

wire_object! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    /// Password limits applied without trimming or rewriting the supplied value.
    pub struct PasswordRules {
        /// Minimum number of Unicode scalar values.
        pub min_codepoints: u16 [minimum = 0, maximum = 65535],
        /// Maximum number of Unicode scalar values.
        pub max_codepoints: u16 [minimum = 0, maximum = 65535],
        /// Maximum size of the password encoded as UTF-8, in bytes.
        pub max_utf8_bytes: u16 [minimum = 0, maximum = 65535],
        /// Counting unit for the minimum and maximum length.
        pub length_unit: PasswordLengthUnit,
    }
}

wire_object! {
    #[derive(Debug)]
    /// Public registration settings and input validation rules.
    pub struct AuthOptionsResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Whether this Server currently accepts new account registrations.
        pub registration_enabled: bool,
        /// Rules required when setting a password.
        pub password_rules: PasswordRules,
        /// Rules required when creating or renaming a network.
        pub network_name_rules: crate::NetworkNameRules,
    }
}

wire_object! {
    /// Credentials for creating a new account without granting network privileges.
    pub struct RegisterRequest {
        /// Login address, normalized by trimming, case folding, and IDNA domain conversion.
        pub email: String,
        /// New password satisfying the advertised password rules, without trimming.
        pub password: WireSecret,
    }
}

wire_object! {
    /// Credentials for issuing a user session for an existing account.
    pub struct LoginRequest {
        /// Login address normalized using the same rules as registration.
        pub email: String,
        /// Account password, compared without trimming or rewriting.
        pub password: WireSecret,
    }
}

wire_object! {
    /// A user account; this view grants no network or instance privileges.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UserView {
        /// Stable account identifier, unchanged by email updates.
        pub id: UserId,
        /// Canonical login identifier, not proof of email ownership.
        pub email: String,
    }
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    /// Non-secret metadata for an opaque user session.
    pub struct SessionMetadata {
        /// Stable identifier of this user session.
        pub id: SessionId,
        /// Time at which the session was issued.
        pub created_at: DateTime<Utc>,
        /// Time at which the session becomes invalid, even if it has been used recently.
        pub expires_at: DateTime<Utc>,
    }
}

wire_object! {
    /// First delivery only. Session reads never return the token.
    pub struct SessionIssuedResponse {
        /// Server-generated request correlation ID, not a registration idempotency key.
        pub request_id: RequestId,
        /// Account authenticated by the issued session.
        pub user: UserView,
        /// Identifier and validity period of the issued session.
        pub session: SessionMetadata,
        /// Opaque bearer credential delivered only in this issuance response.
        pub token: WireSecret,
    }
}

wire_object! {
    #[derive(Debug)]
    /// Authenticated account and current session metadata without its bearer credential.
    pub struct AuthSessionResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Account authenticated by the current session.
        pub user: UserView,
        /// Identifier and validity period of the current session.
        pub session: SessionMetadata,
    }
}

meshway_serde::string_enum! {
/// PKCE transform of the verifier; only S256 is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub enum CodeChallengeMethod {
    S256,
}
}

wire_object! {
    /// A signed-in user's request to let a configured browser client act on their behalf.
    pub struct CreateDelegationRequest {
        /// Name of a delegation client configured on the Server.
        pub client: String,
        /// Absolute `http(s)` URL on the client's origin that will receive the code; no fragment or credentials.
        pub redirect_uri: String,
        /// base64url without padding of the SHA-256 of the client's verifier (43 characters).
        pub code_challenge: String,
        pub code_challenge_method: CodeChallengeMethod,
    }
}

wire_object! {
    /// A one-time authorization code for the client; the Console appends it and the client's own `state` to the redirect target.
    pub struct DelegationCreated {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// The code to deliver as the `code` query parameter; exchangeable once within 60 seconds.
        pub code: WireSecret,
        /// When the code stops being exchangeable.
        pub expires_at: DateTime<Utc>,
    }
}

wire_object! {
    /// A delegation client's exchange of an authorization code for a session.
    pub struct ExchangeDelegationRequest {
        /// The client name the code was issued for.
        pub client: String,
        /// The `code` received in the redirect.
        pub code: WireSecret,
        /// The PKCE verifier whose S256 challenge was sent when authorizing; 43–128 unreserved characters.
        pub code_verifier: WireSecret,
        /// The exact `redirectUri` used when authorizing.
        pub redirect_uri: String,
    }
}

wire_object! {
    /// First and only delivery of a delegated session's bearer token.
    pub struct DelegatedSessionResponse {
        /// Server-generated correlation ID for this HTTP request.
        pub request_id: RequestId,
        /// Account the delegated session acts for.
        pub user: UserView,
        /// Identifier and validity period of the delegated session: one hour, not renewable.
        pub session: SessionMetadata,
        /// Opaque bearer credential for `/api/v1`; the session cannot manage the account, other sessions or delegations.
        pub token: WireSecret,
        /// The client the session was issued to, shown as its label in the account's session list.
        pub client: String,
        /// Origin at which clients reach this Server, or null when the deployment has not configured one.
        #[serde(deserialize_with="nullable")] pub server: Option<String> [required=true],
    }
}
