use super::{auth::bearer, error::HttpError};
use crate::accounts::{Accounts, Secret};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::RequestId;
use meshway_protocol::{
    CreateDelegationRequest, DelegatedSessionResponse, DelegationCreated, ErrorResponse,
    ExchangeDelegationRequest, SessionMetadata, UserView, WireSecret,
};

/// Issue a one-time code letting a configured browser client act for the signed-in user.
///
/// The Console's consent page calls this, then sends the browser to the
/// client's `redirectUri` with `code` and the client's own `state`. The code
/// is bound to this session, the client, the redirect target and the PKCE
/// challenge, and expires after 60 seconds. Delegated sessions may not call
/// this (403); unknown clients, off-origin redirect targets and malformed
/// challenges are `invalid_request`.
#[utoipa::path(post, path = "/api/v1/auth/delegations", operation_id = "createDelegation", tag = "auth", security(("userSession" = [])), request_body = CreateDelegationRequest,
    responses((status = 201, body = DelegationCreated), (status = 400, body = ErrorResponse, description = "invalid_request"), (status = 401, body = ErrorResponse, description = "invalid_session"), (status = 403, body = ErrorResponse, description = "permission_denied"), (status = 413, body = ErrorResponse, description = "request_too_large"), (status = 415, body = ErrorResponse, description = "unsupported_media_type"), (status = 503, body = ErrorResponse, description = "service_unavailable")))]
pub(super) async fn create(
    State(accounts): State<Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<CreateDelegationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<DelegationCreated>), HttpError> {
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let issued = accounts
        .authorize_delegation(
            bearer(&headers, id)?,
            crate::accounts::DelegationAuthorization {
                client: body.client,
                redirect_uri: body.redirect_uri,
                code_challenge: body.code_challenge,
            },
            id,
        )
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok((
        StatusCode::CREATED,
        Json(DelegationCreated {
            request_id: id,
            code: WireSecret::new(issued.code.expose().to_owned()),
            expires_at: issued.expires_at,
        }),
    ))
}

/// Exchange an authorization code for a delegated session.
///
/// Called by the client without a session, from one of the configured client
/// origins. The code is consumed on the first attempt whatever the outcome;
/// a mismatched client, redirect target or verifier, an expired or reused
/// code, or an authorizing session that is no longer valid is
/// `invalid_delegation`. The session lasts one hour, is not renewed, acts as
/// the user except for account, session and delegation management, and
/// appears in the user's session list under the client's name.
#[utoipa::path(post, path = "/api/v1/auth/delegations/token", operation_id = "exchangeDelegation", tag = "auth", request_body = ExchangeDelegationRequest,
    responses((status = 200, body = DelegatedSessionResponse), (status = 400, body = ErrorResponse, description = "invalid_delegation, invalid_request"), (status = 413, body = ErrorResponse, description = "request_too_large"), (status = 415, body = ErrorResponse, description = "unsupported_media_type"), (status = 503, body = ErrorResponse, description = "service_unavailable")))]
pub(super) async fn exchange(
    State(accounts): State<Accounts>,
    State(public_origin): State<super::PublicOrigin>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<ExchangeDelegationRequest>, JsonRejection>,
) -> Result<Json<DelegatedSessionResponse>, HttpError> {
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let delegated = accounts
        .exchange_delegation(
            body.client,
            Secret::new(body.code.expose().to_owned()),
            Secret::new(body.code_verifier.expose().to_owned()),
            body.redirect_uri,
            id,
        )
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    let session = delegated.issued.session;
    Ok(Json(DelegatedSessionResponse {
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
        token: WireSecret::new(delegated.issued.token.expose().to_owned()),
        client: delegated.client,
        server: public_origin
            .0
            .as_deref()
            .map(|origin| origin.as_str().trim_end_matches('/').to_owned()),
    }))
}
