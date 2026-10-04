use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use meshway_model::RequestId;
use meshway_protocol::{EmptyParams, ErrorResponse, Problem};

use crate::accounts::AccountError;
use crate::networks::NetworkError;

pub(super) struct HttpError(ErrorResponse);

impl HttpError {
    pub(super) fn configuration(id: RequestId, error: crate::configuration::Error) -> Self {
        use crate::configuration::Error;
        let (status, problem) = match error {
            Error::InvalidSession => return Self::invalid_session(id),
            Error::NotFound => (
                StatusCode::NOT_FOUND,
                Problem::NotFound {
                    params: EmptyParams {},
                    message: "Configuration not found.".into(),
                },
            ),
            Error::PermissionDenied => (
                StatusCode::FORBIDDEN,
                Problem::PermissionDenied {
                    params: EmptyParams {},
                    message: "Configuration access requires network administrator permission."
                        .into(),
                },
            ),
            Error::Conflict => (
                StatusCode::CONFLICT,
                Problem::RevisionConflict {
                    params: EmptyParams {},
                    message: "Configuration revision changed.".into(),
                },
            ),
            Error::RequestConflict => (
                StatusCode::CONFLICT,
                Problem::RequestConflict {
                    params: EmptyParams {},
                    message: "Request ID was already used with different input.".into(),
                },
            ),
            Error::Invalid(error) => {
                use meshway_configuration::ConfigurationError;
                let (issues, requirements) = match error {
                    ConfigurationError::Invalid(issues) => (issues, Vec::new()),
                    ConfigurationError::PreparationRequired(requirements) => {
                        (Vec::new(), requirements)
                    }
                    ConfigurationError::Parse => (
                        vec![meshway_configuration::ConfigurationIssue {
                            path: "".into(),
                            message: "Invalid JSONC document or configuration field shape.".into(),
                        }],
                        Vec::new(),
                    ),
                    ConfigurationError::Serialization => return Self::unavailable(Some(id)),
                };
                (
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
            Error::Storage | Error::DependencyUnavailable => return Self::unavailable(Some(id)),
        };
        Self::new(id, status, problem)
    }
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

    pub(super) fn account(
        id: RequestId,
        error: AccountError,
        rules: meshway_protocol::PasswordRules,
    ) -> Self {
        let (status, problem) = match error {
            AccountError::RegistrationDisabled => (
                StatusCode::FORBIDDEN,
                Problem::RegistrationDisabled {
                    params: EmptyParams {},
                    message: "Registration is disabled.".into(),
                },
            ),
            AccountError::InvalidEmail => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidRequest {
                    params: EmptyParams {},
                    message: "A valid email address is required.".into(),
                },
            ),
            AccountError::InvalidPassword => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Problem::PasswordWeak {
                    params: rules,
                    message: "Password does not meet the published rules.".into(),
                },
            ),
            AccountError::EmailTaken => (
                StatusCode::CONFLICT,
                Problem::EmailTaken {
                    params: EmptyParams {},
                    message: "Email is already registered.".into(),
                },
            ),
            AccountError::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                Problem::InvalidCredentials {
                    params: EmptyParams {},
                    message: "Invalid credentials.".into(),
                },
            ),
            AccountError::InvalidSession => return Self::invalid_session(id),
            AccountError::InvalidRequest => return Self::invalid_request(id),
            AccountError::InvalidCursor => Self::cursor_problem(),
            AccountError::InvalidCurrentPassword => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidCurrentPassword {
                    params: EmptyParams {},
                    message: "Current password is incorrect.".into(),
                },
            ),
            AccountError::NotFound => (
                StatusCode::NOT_FOUND,
                Problem::NotFound {
                    params: EmptyParams {},
                    message: "Resource not found.".into(),
                },
            ),
            AccountError::InvalidReset => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidPasswordReset {
                    params: EmptyParams {},
                    message: "Recovery credential is invalid or no longer usable.".into(),
                },
            ),
            AccountError::ResetUsed => (
                StatusCode::BAD_REQUEST,
                Problem::PasswordResetUsed {
                    params: EmptyParams {},
                    message: "Recovery credential has already been used.".into(),
                },
            ),
            AccountError::ResetExpired => (
                StatusCode::BAD_REQUEST,
                Problem::PasswordResetExpired {
                    params: EmptyParams {},
                    message: "Recovery credential has expired.".into(),
                },
            ),
            AccountError::ResetReplaced => (
                StatusCode::BAD_REQUEST,
                Problem::PasswordResetReplaced {
                    params: EmptyParams {},
                    message: "Recovery credential has been superseded.".into(),
                },
            ),
            AccountError::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                Problem::Busy {
                    params: EmptyParams {},
                    message: "Account service is busy; retry later.".into(),
                },
            ),
            AccountError::Unavailable | AccountError::Storage => {
                return Self::unavailable(Some(id));
            }
        };
        Self::new(id, status, problem)
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

    fn cursor_problem() -> (StatusCode, Problem) {
        (
            StatusCode::BAD_REQUEST,
            Problem::InvalidCursor {
                params: EmptyParams {},
                message: "Pagination cursor is invalid or expired.".into(),
            },
        )
    }

    pub(super) fn network(id: RequestId, error: NetworkError) -> Self {
        let (status, problem) = match error {
            NetworkError::InvalidSession => return Self::invalid_session(id),
            NetworkError::NotFound => (
                StatusCode::NOT_FOUND,
                Problem::NotFound {
                    params: EmptyParams {},
                    message: "Network not found.".into(),
                },
            ),
            NetworkError::PermissionDenied => (
                StatusCode::FORBIDDEN,
                Problem::PermissionDenied {
                    params: EmptyParams {},
                    message: "This operation requires network administrator permission.".into(),
                },
            ),
            NetworkError::InvalidName | NetworkError::InvalidLimit => {
                return Self::invalid_request(id);
            }
            NetworkError::RequestConflict => (
                StatusCode::CONFLICT,
                Problem::RequestConflict {
                    params: EmptyParams {},
                    message: "Request ID was already used with different input.".into(),
                },
            ),
            NetworkError::NameConflict => (
                StatusCode::CONFLICT,
                Problem::NameConflict {
                    params: EmptyParams {},
                    message: "Network name changed; reload before retrying.".into(),
                },
            ),
            NetworkError::InvalidCursor => Self::cursor_problem(),
            NetworkError::Storage | NetworkError::Unavailable => {
                return Self::unavailable(Some(id));
            }
            NetworkError::VersionConflict => (
                StatusCode::CONFLICT,
                Problem::VersionConflict {
                    params: EmptyParams {},
                    message: "Membership changed; reload before retrying.".into(),
                },
            ),
            NetworkError::RevisionConflict => (
                StatusCode::CONFLICT,
                Problem::RevisionConflict {
                    params: EmptyParams {},
                    message: "Configuration revision changed.".into(),
                },
            ),
            NetworkError::LastOwner => (
                StatusCode::CONFLICT,
                Problem::LastOwner {
                    params: EmptyParams {},
                    message: "The last Owner must remain in the network.".into(),
                },
            ),
            NetworkError::AlreadyMember => (
                StatusCode::CONFLICT,
                Problem::AlreadyMember {
                    params: EmptyParams {},
                    message: "Already a member of this network.".into(),
                },
            ),
            NetworkError::LinkExpired => (
                StatusCode::BAD_REQUEST,
                Problem::LinkExpired {
                    params: EmptyParams {},
                    message: error.to_string(),
                },
            ),
            NetworkError::LinkUsed => (
                StatusCode::BAD_REQUEST,
                Problem::LinkUsed {
                    params: EmptyParams {},
                    message: error.to_string(),
                },
            ),
            NetworkError::LinkRevoked => (
                StatusCode::BAD_REQUEST,
                Problem::LinkRevoked {
                    params: EmptyParams {},
                    message: error.to_string(),
                },
            ),
            NetworkError::LinkIssuerUnavailable => (
                StatusCode::BAD_REQUEST,
                Problem::LinkIssuerUnavailable {
                    params: EmptyParams {},
                    message: error.to_string(),
                },
            ),
            NetworkError::InvalidLink => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidLink {
                    params: EmptyParams {},
                    message: "Join link is invalid or no longer usable.".into(),
                },
            ),
        };
        Self::new(id, status, problem)
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
