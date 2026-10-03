use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
};
use meshway_model::RequestId;
use meshway_protocol::{
    AuthOptionsResponse, AuthSessionResponse, ErrorResponse, LoginRequest, PasswordLengthUnit,
    PasswordRules, RegisterRequest, SessionIssuedResponse, SessionMetadata, UserView, WireSecret,
};

use crate::accounts::{Accounts, AuthenticatedSession, IssuedSession, Secret};

use super::error::HttpError;

fn rules(accounts: &Accounts) -> PasswordRules {
    let rules = accounts.options().password;
    PasswordRules {
        min_codepoints: rules.min_codepoints,
        max_codepoints: rules.max_codepoints,
        max_utf8_bytes: rules.max_utf8_bytes,
        length_unit: PasswordLengthUnit::UnicodeCodepoints,
    }
}

fn session_view(id: RequestId, session: AuthenticatedSession) -> AuthSessionResponse {
    AuthSessionResponse {
        request_id: id,
        user: UserView {
            id: session.user.id,
            email: session.user.email,
        },
        session: SessionMetadata {
            id: session.session_id,
            created_at: session.created_at,
            expires_at: session.expires_at,
        },
    }
}

fn issued_view(id: RequestId, issued: IssuedSession) -> SessionIssuedResponse {
    let view = session_view(id, issued.session);
    SessionIssuedResponse {
        request_id: id,
        user: view.user,
        session: view.session,
        token: WireSecret::new(issued.token.expose().to_owned()),
    }
}

fn bearer(headers: &HeaderMap, id: RequestId) -> Result<&str, HttpError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .ok_or_else(|| HttpError::invalid_session(id))?;
    if values.next().is_some() {
        return Err(HttpError::invalid_session(id));
    }
    let value = value.to_str().map_err(|_| HttpError::invalid_session(id))?;
    let (scheme, token) = value
        .split_once(' ')
        .ok_or_else(|| HttpError::invalid_session(id))?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || token.is_empty()
        || token.bytes().any(|b| b.is_ascii_whitespace() || b == b',')
    {
        return Err(HttpError::invalid_session(id));
    }
    Ok(token)
}

#[utoipa::path(get, path = "/api/v1/auth/options", operation_id = "getAuthOptions", tag = "auth",
    responses((status = 200, body = AuthOptionsResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn options(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
) -> Json<AuthOptionsResponse> {
    Json(AuthOptionsResponse {
        request_id: id,
        registration_enabled: accounts.options().registration_enabled,
        password_rules: rules(&accounts),
    })
}

#[utoipa::path(post, path = "/api/v1/auth/register", operation_id = "register", tag = "auth", request_body = RegisterRequest,
    responses((status = 201, body = SessionIssuedResponse), (status = 400, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 413, body = ErrorResponse), (status = 415, body = ErrorResponse), (status = 422, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn register(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<RegisterRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SessionIssuedResponse>), HttpError> {
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let issued = accounts
        .register(
            body.email,
            Secret::new(body.password.expose().to_owned()),
            id,
        )
        .await
        .map_err(|error| HttpError::account(id, error, rules(&accounts)))?;
    Ok((StatusCode::CREATED, Json(issued_view(id, issued))))
}

#[utoipa::path(post, path = "/api/v1/auth/login", operation_id = "login", tag = "auth", request_body = LoginRequest,
    responses((status = 200, body = SessionIssuedResponse), (status = 400, body = ErrorResponse), (status = 401, body = ErrorResponse), (status = 413, body = ErrorResponse), (status = 415, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn login(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Json<SessionIssuedResponse>, HttpError> {
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let issued = accounts
        .login(
            body.email,
            Secret::new(body.password.expose().to_owned()),
            id,
        )
        .await
        .map_err(|error| HttpError::account(id, error, rules(&accounts)))?;
    Ok(Json(issued_view(id, issued)))
}

#[utoipa::path(get, path = "/api/v1/auth/session", operation_id = "getAuthSession", tag = "auth", security(("userSession" = [])),
    responses((status = 200, body = AuthSessionResponse), (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn session(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<AuthSessionResponse>, HttpError> {
    let session = accounts
        .authenticate(bearer(&headers, id)?)
        .await
        .map_err(|error| HttpError::account(id, error, rules(&accounts)))?;
    Ok(Json(session_view(id, session)))
}

#[utoipa::path(post, path = "/api/v1/auth/logout", operation_id = "logout", tag = "auth", security(("userSession" = [])),
    responses((status = 204, description = "Current session revoked"), (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn logout(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    accounts
        .logout(bearer(&headers, id)?, id)
        .await
        .map_err(|error| HttpError::account(id, error, rules(&accounts)))?;
    Ok(StatusCode::NO_CONTENT)
}
