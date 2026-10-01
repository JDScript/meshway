use axum::{
    Json,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{AgentDnsSnapshot, ErrorResponse, NetworkSnapshot, SaveNetworkRequest};
use utoipa_axum::router::OpenApiRouter;

use crate::{AppState, Error, bearer, json};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_network, save_network))
        .routes(utoipa_axum::routes!(get_agent_dns))
        .layer(DefaultBodyLimit::max(8192))
}

#[utoipa::path(get, path = "/api/v1/network", operation_id = "getNetwork", security(("adminBearer" = [])),
    responses((status = 200, description = "Single-network saved identity, immutable pools and authoritative node DNS intent", body = NetworkSnapshot),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_network(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<NetworkSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.network().map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/network", operation_id = "saveNetwork", security(("adminBearer" = [])), request_body = SaveNetworkRequest,
    responses((status = 200, description = "Saved network intent; not an Agent DNS application receipt", body = NetworkSnapshot),
    (status = 400, description = "Invalid JSON or unknown fields; address pools cannot be edited", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid name, zone, TTL or publication capacity", body = ErrorResponse),
    (status = 413, description = "Request exceeds 8 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn save_network(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<SaveNetworkRequest>, JsonRejection>,
) -> Result<Json<NetworkSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.save_network(&request).map(Json)
        })
        .await
}

#[utoipa::path(get, path = "/api/v1/agent/dns", operation_id = "getAgentDns", security(("nodeBearer" = [])),
    responses((status = 200, description = "Credential-scoped authoritative node DNS bundle; DNS names do not grant access", body = AgentDnsSnapshot),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 422, description = "Invalid inventory or publication capacity", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_agent_dns(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentDnsSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.agent_dns(&token).map(Json))
        .await
}
