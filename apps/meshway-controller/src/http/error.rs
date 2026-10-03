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
            NetworkError::InvalidCursor => (
                StatusCode::BAD_REQUEST,
                Problem::InvalidCursor {
                    params: EmptyParams {},
                    message: "Pagination cursor is invalid or expired.".into(),
                },
            ),
            NetworkError::Storage | NetworkError::Unavailable => {
                return Self::unavailable(Some(id));
            }
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
