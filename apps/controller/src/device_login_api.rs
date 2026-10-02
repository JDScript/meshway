use crate::{AppState, Error, bearer, json};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    AgentNodeResponse, AuthorizeDeviceRequest, AuthorizeDeviceResponse, ErrorResponse,
    ExchangeDeviceRequest,
};
use utoipa_axum::{router::OpenApiRouter, routes};

pub(super) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(authorize_device))
        .routes(routes!(exchange_device))
        .layer(axum::extract::DefaultBodyLimit::max(8192))
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/device-authorizations", operation_id = "authorizeDevice", security(("userSession" = [])), request_body = AuthorizeDeviceRequest,
    params(("network_id" = String, Path, description = "Approved destination network")),
    responses((status = 200, description = "PKCE-bound code; first use expires in five minutes", body = AuthorizeDeviceResponse),
    (status = 404, description = "Approved network not found", body = ErrorResponse),
    (status = 400, description = "Invalid authorization", body = ErrorResponse),
    (status = 401, description = "Admin required", body = ErrorResponse),
    (status = 413, description = "Request too large", body = ErrorResponse),
    (status = 415, description = "JSON required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn authorize_device(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<AuthorizeDeviceRequest>, JsonRejection>,
) -> Result<Json<AuthorizeDeviceResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store
                .authorize_user_device(&token, &network_id, &request)
                .map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/devices/exchange", operation_id = "exchangeDevice", security(("enrollmentBearer" = [])), request_body = ExchangeDeviceRequest,
    responses((status = 200, description = "Same unified active node on bound recovery", body = AgentNodeResponse),
    (status = 400, description = "Invalid exchange; code remains unused", body = ErrorResponse),
    (status = 401, description = "Invalid code, expiry or binding", body = ErrorResponse),
    (status = 403, description = "Bound node revoked", body = ErrorResponse),
    (status = 409, description = "Credential already bound", body = ErrorResponse),
    (status = 422, description = "Capacity exhausted; code remains unused", body = ErrorResponse),
    (status = 413, description = "Request too large", body = ErrorResponse),
    (status = 415, description = "JSON required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn exchange_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ExchangeDeviceRequest>, JsonRejection>,
) -> Result<Json<AgentNodeResponse>, Error> {
    let code = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            Ok(Json(AgentNodeResponse {
                node: store.exchange_device(&code, &request)?,
            }))
        })
        .await
}
