use super::{auth::bearer, error::HttpError};
use crate::accounts::{Accounts, RevokeSessions, Secret};
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use meshway_model::{RequestId, SessionId};
use meshway_protocol::{ErrorResponse, NetworkPageQuery, UserView, management::*};

/// Change the authenticated account's login email.
///
/// Requires the current password and synchronizes bound configuration references while preserving the User ID.
#[utoipa::path(post,path="/api/v1/account/email",operation_id="changeAccountEmail",tag="account",security(("userSession"=[])),request_body=ChangeEmailRequest,responses((status=200,body=AccountResponse),(status=400,body=ErrorResponse,description="invalid_current_password, invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials, invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="email_taken, revision_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="service_unavailable, busy")))]
pub(super) async fn email(
    State(accounts): State<Accounts>,
    State(configurations): State<crate::configuration::Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<ChangeEmailRequest>, JsonRejection>,
) -> Result<Json<AccountResponse>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let user = accounts
        .change_email(
            bearer(&headers, id)?,
            Secret::new(body.password.expose().to_owned()),
            body.email,
            id,
            configurations.committer(),
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(AccountResponse {
        request_id: id,
        user: UserView {
            id: user.id,
            email: user.email,
        },
    }))
}

/// Read the authenticated user's current account metadata.
#[utoipa::path(get,path="/api/v1/account",operation_id="getAccount",tag="account",security(("userSession"=[])),responses((status=200,body=AccountResponse),(status=401,body=ErrorResponse,description="invalid_session"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn account(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<AccountResponse>, HttpError> {
    let user = accounts
        .authenticate(bearer(&headers, id)?)
        .await
        .map_err(|e| HttpError::domain(id, e))?
        .user;
    Ok(Json(AccountResponse {
        request_id: id,
        user: UserView {
            id: user.id,
            email: user.email,
        },
    }))
}

/// List the authenticated user's active sessions.
///
/// Returns newest-first pages with a default limit of 50, without bearer credentials.
#[utoipa::path(get,path="/api/v1/account/sessions",operation_id="listAccountSessions",tag="account",security(("userSession"=[])),params(NetworkPageQuery),responses((status=200,body=AccountSessionsResponse),(status=400,body=ErrorResponse,description="invalid_cursor, invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn sessions(
    State(accounts): State<Accounts>,
    State(sealer): State<std::sync::Arc<crate::secrets::Sealer>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<AccountSessionsResponse>, HttpError> {
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    let page = accounts
        .sessions(
            bearer(&headers, id)?,
            query.limit.unwrap_or(50),
            query.cursor,
            sealer,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(AccountSessionsResponse {
        request_id: id,
        next_cursor: page.next_cursor,
        as_of: page.as_of,
        items: page
            .items
            .into_iter()
            .map(|s| AccountSessionView {
                id: s.id,
                created_at: s.created_at,
                expires_at: s.expires_at,
                last_used_at: s.last_used_at,
                client_label: s.client_label,
                current: s.current,
                delegation_client: s.delegation_client,
            })
            .collect(),
    }))
}

async fn revoke(
    accounts: Accounts,
    id: RequestId,
    headers: HeaderMap,
    target: RevokeSessions,
) -> Result<StatusCode, HttpError> {
    accounts
        .revoke_sessions(bearer(&headers, id)?, target, id)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Revoke one session belonging to the authenticated user.
#[utoipa::path(post,path="/api/v1/account/sessions/{sessionId}/revoke",operation_id="revokeAccountSession",tag="account",security(("userSession"=[])),params(("sessionId"=SessionId,Path)),responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn revoke_one(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<SessionId>, PathRejection>,
) -> Result<StatusCode, HttpError> {
    let Path(session) = path.map_err(|_| HttpError::invalid_request(id))?;
    revoke(accounts, id, headers, RevokeSessions::One(session)).await
}
/// Revoke the authenticated user's sessions except the current session.
#[utoipa::path(post,path="/api/v1/account/sessions/revoke-others",operation_id="revokeOtherAccountSessions",tag="account",security(("userSession"=[])),responses((status=204),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn revoke_others(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    revoke(accounts, id, headers, RevokeSessions::Others).await
}
/// Revoke all sessions belonging to the authenticated user, including the current session.
#[utoipa::path(post,path="/api/v1/account/sessions/revoke-all",operation_id="revokeAllAccountSessions",tag="account",security(("userSession"=[])),responses((status=204),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn revoke_all(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    revoke(accounts, id, headers, RevokeSessions::All).await
}

/// Change the authenticated user's password using the current password.
///
/// The replacement must satisfy the advertised password rules.
#[utoipa::path(post,path="/api/v1/account/password",operation_id="changeAccountPassword",tag="account",security(("userSession"=[])),request_body=ChangePasswordRequest,responses((status=204),(status=400,body=ErrorResponse,description="invalid_current_password, invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials, invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="password_weak"),(status=503,body=ErrorResponse,description="service_unavailable, busy")))]
pub(super) async fn password(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> Result<StatusCode, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    accounts
        .change_password(
            bearer(&headers, id)?,
            Secret::new(body.current_password.expose().to_owned()),
            Secret::new(body.password.expose().to_owned()),
            id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Inspect a password reset credential without consuming it.
///
/// Accepts the operator-issued credential in the request body and returns its expiry without account details.
#[utoipa::path(post,path="/api/v1/auth/password-reset/inspect",operation_id="inspectPasswordReset",tag="auth",request_body=PasswordResetInspectRequest,responses((status=200,body=PasswordResetInspection),(status=400,body=ErrorResponse,description="invalid_password_reset, password_reset_used, password_reset_expired, password_reset_replaced, invalid_request"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn reset_inspect(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<PasswordResetInspectRequest>, JsonRejection>,
) -> Result<Json<PasswordResetInspection>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let reset = accounts
        .inspect_password_reset(body.token.expose())
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(PasswordResetInspection {
        request_id: id,
        expires_at: reset.expires_at,
    }))
}
/// Redeem a one-time password reset credential and replace the account password.
///
/// Validates the credential binding and password rules, consumes the reset, and revokes old user sessions.
#[utoipa::path(post,path="/api/v1/auth/password-reset/redeem",operation_id="redeemPasswordReset",tag="auth",request_body=PasswordResetRedeemRequest,responses((status=204),(status=400,body=ErrorResponse,description="invalid_password_reset, password_reset_used, password_reset_expired, password_reset_replaced, invalid_request"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="password_weak"),(status=503,body=ErrorResponse,description="service_unavailable, busy")))]
pub(super) async fn reset_redeem(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<PasswordResetRedeemRequest>, JsonRejection>,
) -> Result<StatusCode, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    accounts
        .redeem_password_reset(
            body.token.expose(),
            Secret::new(body.password.expose().to_owned()),
            id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
