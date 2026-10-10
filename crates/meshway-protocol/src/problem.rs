use meshway_model::RequestId;
use serde::{Deserialize, Serialize};
use utoipa::{PartialSchema, ToSchema};

use crate::{PasswordRules, object::wire_object};
use meshway_model::wire::required_nullable as required_request_id;

wire_object! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    /// Empty parameter object for problem codes that expose no additional context.
    pub struct EmptyParams {}
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    /// Typed configuration errors and preparation requirements reported by validation.
    pub struct ConfigurationValidationParams {
        /// Validation issues located by JSON Pointer in the submitted input.
        pub issues: Vec<meshway_configuration::ConfigurationIssue>,
        /// Material, address, or other preparation requirements for the submitted configuration.
        pub requirements: Vec<meshway_configuration::PreparationRequirement>,
    }
}

// utoipa's tagged enum derive does not close each variant object for
// serde(deny_unknown_fields). Keep the wire enum and closed schema in one list.
macro_rules! problems {
    ($($(#[doc = $doc:literal])* $variant:ident($params:ty) = $code:literal),+ $(,)?) => {
        /// Closed error variants; code determines the exact params type.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(tag = "code", deny_unknown_fields)]
        pub enum Problem {
            $($(#[doc = $doc])* #[serde(rename = $code)] $variant {
                /// Structured context defined by this problem code, or an empty object when none is exposed.
                params: $params,
                /// Safe fallback text. Clients branch on code.
                message: String,
            }),+
        }

        impl<'de> Deserialize<'de> for Problem {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(tag = "code", deny_unknown_fields)]
                enum Fields {
                    $(#[serde(rename = $code)] $variant { params: $params, message: String }),+
                }
                Ok(match Fields::deserialize(crate::object::MapOnly(deserializer))? {
                    $(Fields::$variant { params, message } => Self::$variant { params, message }),+
                })
            }
        }

        impl PartialSchema for Problem {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                use utoipa::openapi::schema::{AdditionalProperties, ObjectBuilder, OneOfBuilder, Type};
                // Each variant's doc comment describes its code in the contract.
                OneOfBuilder::new()
                    $(.item(ObjectBuilder::new()
                        .description(Some(concat!($($doc, "\n"),*).trim()).filter(|text| !text.is_empty()))
                        .property("code", ObjectBuilder::new().schema_type(Type::String).enum_values(Some([$code])))
                        .property("params", utoipa::openapi::schema::RefBuilder::new()
                            .ref_location_from_schema_name(<$params>::name())
                            .description(Some("Structured context defined by this problem code, or an empty object when none is exposed.")))
                        .property("message", ObjectBuilder::new().schema_type(Type::String)
                            .description(Some("Safe fallback text. Clients branch on code.")))
                        .required("code")
                        .required("params")
                        .required("message")
                        .additional_properties(Some(AdditionalProperties::FreeForm(false)))))+
                    .into()
            }
        }

        impl ToSchema for Problem {
            fn schemas(schemas: &mut Vec<(String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>)>) {
                $(
                    if !schemas.iter().any(|(name, _)| name == &<$params>::name()) {
                        schemas.push((<$params>::name().into_owned(), <$params>::schema()));
                        <$params>::schemas(schemas);
                    }
                )+
            }
        }
    };
}

problems! {
    /// The supplied login or Node credentials fail authentication; returns HTTP 401 without disclosing credential details.
    InvalidCredentials(EmptyParams) = "invalid_credentials",
    /// This Node's registered identity or key is no longer eligible; returns HTTP 403 and invalidates its held netmap.
    NodeIdentityRejected(EmptyParams) = "node_identity_rejected",
    /// The supplied current password is incorrect; returns HTTP 400.
    InvalidCurrentPassword(EmptyParams) = "invalid_current_password",
    /// The password recovery credential is invalid or no longer usable; returns HTTP 400.
    InvalidPasswordReset(EmptyParams) = "invalid_password_reset",
    /// The verified password recovery credential has already been consumed; returns HTTP 400.
    PasswordResetUsed(EmptyParams) = "password_reset_used",
    /// The verified password recovery credential has reached its expiry time; returns HTTP 400.
    PasswordResetExpired(EmptyParams) = "password_reset_expired",
    /// The verified password recovery credential was superseded or its account credential version changed; returns HTTP 400.
    PasswordResetReplaced(EmptyParams) = "password_reset_replaced",
    /// The session token is missing, unknown, expired, revoked, or otherwise invalid; returns HTTP 401 and requires signing in again.
    InvalidSession(EmptyParams) = "invalid_session",
    /// Public account creation is disabled by the Server; returns HTTP 403.
    RegistrationDisabled(EmptyParams) = "registration_disabled",
    /// The requested email address is already registered; returns HTTP 409 without disclosing account details.
    EmailTaken(EmptyParams) = "email_taken",
    /// The proposed password fails the published rules; returns HTTP 422 with PasswordRules in params.
    PasswordWeak(PasswordRules) = "password_weak",
    /// The request JSON, parameters, or input values are invalid; returns HTTP 400.
    InvalidRequest(EmptyParams) = "invalid_request",
    /// The request body exceeds the allowed size; returns HTTP 413.
    RequestTooLarge(EmptyParams) = "request_too_large",
    /// The request does not supply the required JSON content type; returns HTTP 415.
    UnsupportedMediaType(EmptyParams) = "unsupported_media_type",
    /// The Server cannot admit work within its current computation budget; returns HTTP 503 with Retry-After: 1, so retry later.
    Busy(EmptyParams) = "busy",
    /// A required dependency, runtime, storage operation, or internal Server operation is unavailable or failed; returns HTTP 503.
    ServiceUnavailable(EmptyParams) = "service_unavailable",
    /// The requested resource or route cannot be found; returns HTTP 404.
    NotFound(EmptyParams) = "not_found",
    /// The requested HTTP method is not supported by the route; returns HTTP 405.
    MethodNotAllowed(EmptyParams) = "method_not_allowed",
    /// The authenticated principal lacks permission for the requested operation; returns HTTP 403.
    PermissionDenied(EmptyParams) = "permission_denied",
    /// A request ID was already used with different input; returns HTTP 409 independently of generation, binding, or revision conflicts.
    RequestConflict(EmptyParams) = "request_id_conflict",
    /// The request does not match the current generation or binding; returns HTTP 409 rather than indicating request-ID reuse.
    BindingConflict(EmptyParams) = "binding_conflict",
    /// The receipt's instance, generation, Server boot, or instance deadline no longer matches; returns HTTP 409 and requires fencing before obtaining valid eligibility again.
    ReceiptInstanceMismatch(EmptyParams) = "receipt_instance_mismatch",
    /// The receipt netmap is neither current nor a valid historical netmap of this Node; returns HTTP 409 and requires fencing and entering the blocked state.
    ReceiptTargetMismatch(EmptyParams) = "receipt_target_mismatch",
    /// An applied receipt names a netmap that a newer serial already replaced; returns HTTP 409. Discard the rejected receipt, synchronize, and create a new installation attempt for the current netmap.
    ReceiptNetmapSuperseded(EmptyParams) = "receipt_netmap_superseded",
    /// The receipt observation time is out of bounds or applied lacks an earlier matching saved receipt; returns HTTP 409. Discard the rejected receipt, synchronize, and confirm saved again in a new installation attempt.
    ReceiptPrecondition(EmptyParams) = "receipt_precondition",
    /// A receipt reuses an instance/netmap/attempt/sequence identity with different content or has incompatible phase and failure values; returns HTTP 409. Discard it without rewriting the conflicting receipt and create a new installation attempt.
    ReceiptContentConflict(EmptyParams) = "receipt_content_conflict",
    /// The resource name changed since the client read it; returns HTTP 409, so reload before retrying.
    NameConflict(EmptyParams) = "name_conflict",
    /// The pagination cursor is invalid or expired; returns HTTP 400.
    InvalidCursor(EmptyParams) = "invalid_cursor",
    /// The resource version changed since the client read it; returns HTTP 409, so reload before retrying.
    VersionConflict(EmptyParams) = "version_conflict",
    /// The operation would remove or demote the network's last Owner; returns HTTP 409.
    LastOwner(EmptyParams) = "last_owner",
    /// The user attempting to join already belongs to the network; returns HTTP 409.
    AlreadyMember(EmptyParams) = "already_member",
    /// The join token is invalid or no longer usable; returns HTTP 400 without exposing unverified link details.
    InvalidLink(EmptyParams) = "invalid_join_link",
    /// The verified join token has reached its expiry time; returns HTTP 400.
    LinkExpired(EmptyParams) = "join_link_expired",
    /// The verified one-time join token has already been consumed; returns HTTP 400.
    LinkUsed(EmptyParams) = "join_link_used",
    /// The verified join token was revoked; returns HTTP 400.
    LinkRevoked(EmptyParams) = "join_link_revoked",
    /// The verified join token's issuer is no longer authorized to grant membership; returns HTTP 400.
    LinkIssuerUnavailable(EmptyParams) = "join_link_issuer_unavailable",
    /// The delegation code is unknown, expired, already exchanged, or does not match its client, redirect target or PKCE verifier; returns HTTP 400 and the client must start a new authorization.
    InvalidDelegation(EmptyParams) = "invalid_delegation",
    /// The expected configuration revision no longer matches the current revision; returns HTTP 409, so re-read the baseline and revalidate the preserved draft before applying again.
    RevisionConflict(EmptyParams) = "revision_conflict",
    /// Configuration validation fails or required preparation is incomplete; returns HTTP 422 with ConfigurationValidationParams containing issues and requirements.
    ConfigurationInvalid(ConfigurationValidationParams) = "configuration_invalid",
    /// The current lifecycle state or sequence does not permit the operation; returns HTTP 409.
    StateConflict(EmptyParams) = "state_conflict",
    /// The State report's execution scope changed; returns HTTP 409, so refresh the scope and observe again.
    StateScopeChanged(EmptyParams) = "state_scope_changed",
    /// The operation exceeds the State resource budget; returns HTTP 429.
    StateLimit(EmptyParams) = "state_limit",
    /// The operation exceeds the observation budget; returns HTTP 429.
    ObservationLimit(EmptyParams) = "observation_limit",
}

wire_object! {
    #[derive(Debug)]
    /// HTTP failure envelope with a stable problem code and safe fallback text.
    pub struct ErrorResponse {
        /// Null only when secure request-ID generation itself is unavailable.
        #[serde(deserialize_with = "required_request_id")]
        pub request_id: Option<RequestId> [required = true],
        /// HTTP status code of the failed response.
        pub status: u16 [minimum = 0, maximum = 65535],
        /// Typed failure whose code determines the available parameters.
        pub problem: Problem,
    }
}
