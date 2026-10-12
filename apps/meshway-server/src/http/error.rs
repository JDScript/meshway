//! The only place a domain [`Error`] becomes a wire Problem. Messages are fixed
//! and never carry stored values; infrastructure causes stay server-side.

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use meshway_configuration::ConfigurationError;
use meshway_model::RequestId;
use meshway_protocol::{EmptyParams, ErrorResponse, Problem};

use crate::error::Error;

pub(super) struct HttpError(ErrorResponse);

impl HttpError {
    pub(super) fn new(id: RequestId, status: StatusCode, problem: Problem) -> Self {
        Self(ErrorResponse {
            request_id: Some(id),
            status: status.as_u16(),
            problem,
        })
    }

    pub(super) fn unavailable(id: Option<RequestId>) -> Self {
        Self(ErrorResponse {
            request_id: id,
            status: 503,
            problem: Problem::ServiceUnavailable {
                params: EmptyParams {},
                message: "Service is unavailable.".into(),
            },
        })
    }

    pub(super) fn invalid_session(id: RequestId) -> Self {
        Self::new(
            id,
            StatusCode::UNAUTHORIZED,
            Problem::InvalidSession {
                params: EmptyParams {},
                message: "Invalid or expired session.".into(),
            },
        )
    }

    pub(super) fn invalid_request(id: RequestId) -> Self {
        Self::new(
            id,
            StatusCode::BAD_REQUEST,
            Problem::InvalidRequest {
                params: EmptyParams {},
                message: "Invalid request parameters.".into(),
            },
        )
    }

    pub(super) fn json(id: RequestId, rejection: JsonRejection) -> Self {
        let (status, problem) = match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Problem::RequestTooLarge {
                    params: EmptyParams {},
                    message: "Request body exceeds the allowed size.".into(),
                },
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Problem::UnsupportedMediaType {
                    params: EmptyParams {},
                    message: "A JSON request body is required.".into(),
                },
            ),
            _ => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidRequest {
                    params: EmptyParams {},
                    message: "Invalid JSON request.".into(),
                },
            ),
        };
        Self::new(id, status, problem)
    }

    /// Maps a use-case outcome to its Problem. Node credential failures use
    /// `invalid_credentials`; user session failures use `invalid_session`.
    pub(super) fn domain(id: RequestId, error: Error) -> Self {
        use StatusCode as S;
        let empty = EmptyParams {};
        let simple = |status: StatusCode, problem: Problem| Self::new(id, status, problem);
        match error {
            Error::InvalidSession => Self::invalid_session(id),
            Error::InvalidCredentials => simple(
                S::UNAUTHORIZED,
                Problem::InvalidCredentials {
                    params: empty,
                    message: "Invalid credentials.".into(),
                },
            ),
            Error::NodeIdentityRejected => simple(
                StatusCode::FORBIDDEN,
                Problem::NodeIdentityRejected {
                    params: EmptyParams {},
                    message: "Node identity is no longer eligible".into(),
                },
            ),
            Error::NetmapWithdrawn => simple(
                StatusCode::FORBIDDEN,
                Problem::NetmapWithdrawn {
                    params: EmptyParams {},
                    message: "The Server holds no netmap for this Node".into(),
                },
            ),
            Error::PermissionDenied => simple(
                S::FORBIDDEN,
                Problem::PermissionDenied {
                    params: empty,
                    message: "This operation is not permitted for the current principal.".into(),
                },
            ),
            Error::RegistrationDisabled => simple(
                S::FORBIDDEN,
                Problem::RegistrationDisabled {
                    params: empty,
                    message: "Registration is disabled.".into(),
                },
            ),
            Error::NotFound => simple(
                S::NOT_FOUND,
                Problem::NotFound {
                    params: empty,
                    message: "Resource not found.".into(),
                },
            ),
            Error::CatalogNotReady => simple(
                S::CONFLICT,
                Problem::RuleSetCatalogNotReady {
                    params: empty,
                    message: "The provider's catalog has not been verified by this Server yet."
                        .into(),
                },
            ),
            Error::RevisionConflict => simple(
                S::CONFLICT,
                Problem::RevisionConflict {
                    params: empty,
                    message: "Configuration revision changed.".into(),
                },
            ),
            Error::RequestConflict => simple(
                S::CONFLICT,
                Problem::RequestConflict {
                    params: empty,
                    message: "Request ID was already used with different input.".into(),
                },
            ),
            Error::BindingConflict => simple(
                S::CONFLICT,
                Problem::BindingConflict {
                    params: empty,
                    message: "Request conflicts with the current generation or binding.".into(),
                },
            ),
            Error::ReceiptInstanceMismatch => simple(
                S::CONFLICT,
                Problem::ReceiptInstanceMismatch {
                    params: empty,
                    message: "Receipt instance is no longer current.".into(),
                },
            ),
            Error::ReceiptTargetMismatch => simple(
                S::CONFLICT,
                Problem::ReceiptTargetMismatch {
                    params: empty,
                    message: "Receipt target does not belong to this node.".into(),
                },
            ),
            Error::ReceiptNetmapSuperseded => simple(
                S::CONFLICT,
                Problem::ReceiptNetmapSuperseded {
                    params: empty,
                    message: "Applied receipt names a superseded netmap; synchronize.".into(),
                },
            ),
            Error::ReceiptPrecondition => simple(
                S::CONFLICT,
                Problem::ReceiptPrecondition {
                    params: empty,
                    message: "Receipt timing or saved prerequisite is not satisfied.".into(),
                },
            ),
            Error::ReceiptContentConflict => simple(
                S::CONFLICT,
                Problem::ReceiptContentConflict {
                    params: empty,
                    message: "Receipt sequence was already used with different content.".into(),
                },
            ),
            Error::NameConflict => simple(
                S::CONFLICT,
                Problem::NameConflict {
                    params: empty,
                    message: "Name changed; reload before retrying.".into(),
                },
            ),
            Error::VersionConflict => simple(
                S::CONFLICT,
                Problem::VersionConflict {
                    params: empty,
                    message: "Resource changed; reload before retrying.".into(),
                },
            ),
            Error::StateConflict => simple(
                S::CONFLICT,
                Problem::StateConflict {
                    params: empty,
                    message: "Lifecycle state or sequence conflicts.".into(),
                },
            ),
            Error::StateScopeChanged => simple(
                S::CONFLICT,
                Problem::StateScopeChanged {
                    params: empty,
                    message: "State report execution scope changed; refresh and observe again."
                        .into(),
                },
            ),
            Error::LastOwner => simple(
                S::CONFLICT,
                Problem::LastOwner {
                    params: empty,
                    message: "The last Owner must remain in the network.".into(),
                },
            ),
            Error::AlreadyMember => simple(
                S::CONFLICT,
                Problem::AlreadyMember {
                    params: empty,
                    message: "Already a member of this network.".into(),
                },
            ),
            Error::InvalidRequest | Error::InvalidName | Error::InvalidLimit => {
                Self::invalid_request(id)
            }
            Error::InvalidEmail => simple(
                S::BAD_REQUEST,
                Problem::InvalidRequest {
                    params: empty,
                    message: "A valid email address is required.".into(),
                },
            ),
            Error::InvalidPassword(rules) => match super::auth::rules(rules, id) {
                Ok(rules) => simple(
                    S::UNPROCESSABLE_ENTITY,
                    Problem::PasswordWeak {
                        params: rules,
                        message: "Password does not meet the published rules.".into(),
                    },
                ),
                Err(error) => error,
            },
            Error::InvalidCurrentPassword => simple(
                S::BAD_REQUEST,
                Problem::InvalidCurrentPassword {
                    params: empty,
                    message: "Current password is incorrect.".into(),
                },
            ),
            Error::EmailTaken => simple(
                S::CONFLICT,
                Problem::EmailTaken {
                    params: empty,
                    message: "Email is already registered.".into(),
                },
            ),
            Error::InvalidCursor => simple(
                S::BAD_REQUEST,
                Problem::InvalidCursor {
                    params: empty,
                    message: "Pagination cursor is invalid or expired.".into(),
                },
            ),
            Error::InvalidConfiguration(error) => Self::configuration(id, error),
            Error::RoutingProfileUnavailable(params) => simple(
                S::UNPROCESSABLE_ENTITY,
                Problem::RoutingProfileUnavailable {
                    params,
                    message: "The routing profile is not selectable for this Node.".into(),
                },
            ),
            Error::InvalidReset => simple(
                S::BAD_REQUEST,
                Problem::InvalidPasswordReset {
                    params: empty,
                    message: "Recovery credential is invalid or no longer usable.".into(),
                },
            ),
            Error::InvalidDelegation => simple(
                S::BAD_REQUEST,
                Problem::InvalidDelegation {
                    params: empty,
                    message: "Delegation code is invalid or no longer usable.".into(),
                },
            ),
            Error::ResetUsed => simple(
                S::BAD_REQUEST,
                Problem::PasswordResetUsed {
                    params: empty,
                    message: "Recovery credential has already been used.".into(),
                },
            ),
            Error::ResetExpired => simple(
                S::BAD_REQUEST,
                Problem::PasswordResetExpired {
                    params: empty,
                    message: "Recovery credential has expired.".into(),
                },
            ),
            Error::ResetReplaced => simple(
                S::BAD_REQUEST,
                Problem::PasswordResetReplaced {
                    params: empty,
                    message: "Recovery credential has been superseded.".into(),
                },
            ),
            Error::InvalidLink => simple(
                S::BAD_REQUEST,
                Problem::InvalidLink {
                    params: empty,
                    message: "Join link is invalid or no longer usable.".into(),
                },
            ),
            Error::LinkExpired => simple(
                S::BAD_REQUEST,
                Problem::LinkExpired {
                    params: empty,
                    message: "Join link has expired.".into(),
                },
            ),
            Error::LinkUsed => simple(
                S::BAD_REQUEST,
                Problem::LinkUsed {
                    params: empty,
                    message: "Join link has been used.".into(),
                },
            ),
            Error::LinkRevoked => simple(
                S::BAD_REQUEST,
                Problem::LinkRevoked {
                    params: empty,
                    message: "Join link was revoked.".into(),
                },
            ),
            Error::LinkIssuerUnavailable => simple(
                S::BAD_REQUEST,
                Problem::LinkIssuerUnavailable {
                    params: empty,
                    message: "Join link issuer is no longer authorized.".into(),
                },
            ),
            Error::Busy => simple(
                S::SERVICE_UNAVAILABLE,
                Problem::Busy {
                    params: empty,
                    message: "Service is busy; retry later.".into(),
                },
            ),
            Error::StateLimit => simple(
                S::TOO_MANY_REQUESTS,
                Problem::StateLimit {
                    params: empty,
                    message: "State resource budget exceeded.".into(),
                },
            ),
            Error::ObservationLimit => simple(
                S::TOO_MANY_REQUESTS,
                Problem::ObservationLimit {
                    params: empty,
                    message: "Observation budget exceeded.".into(),
                },
            ),
            Error::DependencyUnavailable
            | Error::Unavailable(_)
            | Error::Storage(_)
            | Error::Internal(_) => {
                // The response says only "unavailable"; the cause is a safe
                // category label and belongs in the operator's log.
                tracing::error!(error = %error, "Request failed on the Server side");
                Self::unavailable(Some(id))
            }
        }
    }

    fn configuration(id: RequestId, error: ConfigurationError) -> Self {
        let (issues, requirements) = match error {
            ConfigurationError::Invalid(issues) => (issues, Vec::new()),
            ConfigurationError::PreparationRequired(requirements) => (Vec::new(), requirements),
            ConfigurationError::Parse => (
                vec![meshway_configuration::ConfigurationIssue::new(
                    "",
                    meshway_configuration::IssueCode::IncompleteVersion,
                )],
                Vec::new(),
            ),
            ConfigurationError::Serialization => return Self::unavailable(Some(id)),
        };
        Self::new(
            id,
            StatusCode::UNPROCESSABLE_ENTITY,
            Problem::ConfigurationInvalid {
                params: meshway_protocol::ConfigurationValidationParams {
                    issues,
                    requirements,
                },
                message: "Configuration validation failed.".into(),
            },
        )
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let busy = matches!(self.0.problem, Problem::Busy { .. });
        let mut response = (status, Json(self.0)).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer realm=\"meshway\""),
            );
        }
        if busy {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
        }
        response
    }
}

/// One sample response per status/code pair the Server produces: every
/// `Error` variant through `domain`, plus the rejections raised by body
/// extraction and routing before any use case runs. Operations declare the
/// codes they return in their error response descriptions; a test checks each
/// declared pair against this and the contract carries the sample as example.
pub fn problem_samples() -> Vec<ProblemSample> {
    use crate::error::{StorageCause, Unavailability};
    let samples = [
        Error::InvalidSession,
        Error::InvalidCredentials,
        Error::NodeIdentityRejected,
        Error::NetmapWithdrawn,
        Error::PermissionDenied,
        Error::RegistrationDisabled,
        Error::NotFound,
        Error::RevisionConflict,
        Error::RequestConflict,
        Error::NameConflict,
        Error::VersionConflict,
        Error::BindingConflict,
        Error::ReceiptInstanceMismatch,
        Error::ReceiptTargetMismatch,
        Error::ReceiptNetmapSuperseded,
        Error::ReceiptPrecondition,
        Error::ReceiptContentConflict,
        Error::StateConflict,
        Error::CatalogNotReady,
        Error::StateScopeChanged,
        Error::LastOwner,
        Error::AlreadyMember,
        Error::InvalidRequest,
        Error::InvalidEmail,
        Error::InvalidPassword(
            crate::accounts::Options::new(true, std::time::Duration::from_secs(60))
                .expect("valid options")
                .password,
        ),
        Error::InvalidCurrentPassword,
        Error::EmailTaken,
        Error::InvalidName,
        Error::InvalidLimit,
        Error::InvalidCursor,
        Error::InvalidConfiguration(meshway_configuration::ConfigurationError::Invalid(
            Vec::new(),
        )),
        Error::RoutingProfileUnavailable(meshway_protocol::RoutingProfileUnavailableParams {
            reason: meshway_configuration::selection::SelectionUnavailable::NotAllowed,
            allowed: vec!["mesh".into(), "work".into()],
        }),
        Error::InvalidReset,
        Error::InvalidDelegation,
        Error::ResetUsed,
        Error::ResetExpired,
        Error::ResetReplaced,
        Error::InvalidLink,
        Error::LinkExpired,
        Error::LinkUsed,
        Error::LinkRevoked,
        Error::LinkIssuerUnavailable,
        Error::Busy,
        Error::StateLimit,
        Error::ObservationLimit,
        Error::DependencyUnavailable,
        Error::Unavailable(Unavailability::Closed),
        Error::Storage(StorageCause::Decode("sample")),
        Error::Internal("sample"),
    ];
    let id = RequestId::new(uuid::Uuid::from_u128(1)).expect("nonzero request ID");
    let empty = || EmptyParams {};
    samples
        .into_iter()
        .map(|error| HttpError::domain(id, error))
        .chain([
            HttpError::new(
                id,
                StatusCode::PAYLOAD_TOO_LARGE,
                Problem::RequestTooLarge {
                    params: empty(),
                    message: "Request body exceeds the allowed size.".into(),
                },
            ),
            HttpError::new(
                id,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Problem::UnsupportedMediaType {
                    params: empty(),
                    message: "A JSON request body is required.".into(),
                },
            ),
            HttpError::new(
                id,
                StatusCode::METHOD_NOT_ALLOWED,
                Problem::MethodNotAllowed {
                    params: empty(),
                    message: "Method not allowed.".into(),
                },
            ),
        ])
        .map(|HttpError(response)| {
            let body = serde_json::to_value(&response).expect("error response serializes");
            let code = body["problem"]["code"]
                .as_str()
                .expect("problem code")
                .to_owned();
            ProblemSample {
                status: response.status,
                code,
                body,
            }
        })
        .collect()
}

pub struct ProblemSample {
    pub status: u16,
    pub code: String,
    pub body: serde_json::Value,
}
