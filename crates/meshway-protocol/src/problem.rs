use meshway_model::RequestId;
use serde::{Deserialize, Serialize};
use utoipa::{PartialSchema, ToSchema};

use crate::{PasswordRules, object::wire_object};

wire_object! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EmptyParams {}
}

wire_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ConfigurationValidationParams {
        pub issues: Vec<meshway_configuration::ConfigurationIssue>,
        pub requirements: Vec<meshway_configuration::PreparationRequirement>,
    }
}

// utoipa's tagged enum derive does not close each variant object for
// serde(deny_unknown_fields). Keep the wire enum and closed schema in one list.
macro_rules! problems {
    ($($variant:ident($params:ty) = $code:literal),+ $(,)?) => {
        /// Closed error variants; code determines the exact params type.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(tag = "code", deny_unknown_fields)]
        pub enum Problem {
            $(#[serde(rename = $code)] $variant {
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
                use utoipa::openapi::{Ref, schema::{AdditionalProperties, ObjectBuilder, OneOfBuilder, Type}};
                OneOfBuilder::new()
                    $(.item(ObjectBuilder::new()
                        .property("code", ObjectBuilder::new().schema_type(Type::String).enum_values(Some([$code])))
                        .property("params", Ref::from_schema_name(<$params>::name()))
                        .property("message", ObjectBuilder::new().schema_type(Type::String))
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
    InvalidCredentials(EmptyParams) = "invalid_credentials",
    InvalidCurrentPassword(EmptyParams) = "invalid_current_password",
    InvalidPasswordReset(EmptyParams) = "invalid_password_reset",
    PasswordResetUsed(EmptyParams) = "password_reset_used",
    PasswordResetExpired(EmptyParams) = "password_reset_expired",
    PasswordResetReplaced(EmptyParams) = "password_reset_replaced",
    InvalidSession(EmptyParams) = "invalid_session",
    RegistrationDisabled(EmptyParams) = "registration_disabled",
    EmailTaken(EmptyParams) = "email_taken",
    PasswordWeak(PasswordRules) = "password_weak",
    InvalidRequest(EmptyParams) = "invalid_request",
    RequestTooLarge(EmptyParams) = "request_too_large",
    UnsupportedMediaType(EmptyParams) = "unsupported_media_type",
    Busy(EmptyParams) = "busy",
    ServiceUnavailable(EmptyParams) = "service_unavailable",
    NotFound(EmptyParams) = "not_found",
    MethodNotAllowed(EmptyParams) = "method_not_allowed",
    PermissionDenied(EmptyParams) = "permission_denied",
    RequestConflict(EmptyParams) = "request_id_conflict",
    NameConflict(EmptyParams) = "name_conflict",
    InvalidCursor(EmptyParams) = "invalid_cursor",
    VersionConflict(EmptyParams) = "version_conflict",
    LastOwner(EmptyParams) = "last_owner",
    AlreadyMember(EmptyParams) = "already_member",
    InvalidLink(EmptyParams) = "invalid_join_link",
    LinkExpired(EmptyParams) = "join_link_expired",
    LinkUsed(EmptyParams) = "join_link_used",
    LinkRevoked(EmptyParams) = "join_link_revoked",
    LinkIssuerUnavailable(EmptyParams) = "join_link_issuer_unavailable",
    RevisionConflict(EmptyParams) = "revision_conflict",
    ConfigurationInvalid(ConfigurationValidationParams) = "configuration_invalid",
    StateConflict(EmptyParams) = "state_conflict",
    StateScopeChanged(EmptyParams) = "state_scope_changed",
    StateLimit(EmptyParams) = "state_limit",
    ObservationLimit(EmptyParams) = "observation_limit",
}

wire_object! {
    #[derive(Debug)]
    pub struct ErrorResponse {
        /// Null only when secure request-ID generation itself is unavailable.
        #[serde(deserialize_with = "required_request_id")]
        pub request_id: Option<RequestId> [required = true],
        pub status: u16 [minimum = 0, maximum = 65535],
        pub problem: Problem,
    }
}

fn required_request_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<RequestId>, D::Error> {
    Option::<RequestId>::deserialize(deserializer)
}
