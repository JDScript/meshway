use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse, ConfigurationSnapshot,
    ErrorResponse, PolicySnapshot, PreviewPolicyRequest, PreviewPolicyResponse, RevokeNodeResponse,
    SavePolicyRequest, SetNodeCountryRequest,
};
use utoipa_axum::router::OpenApiRouter;

use crate::{AppState, Error, bearer, json};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_policy))
        .routes(utoipa_axum::routes!(preview_policy))
        .routes(utoipa_axum::routes!(save_policy))
        .routes(utoipa_axum::routes!(set_node_country))
        .routes(utoipa_axum::routes!(get_agent_configuration))
        .routes(utoipa_axum::routes!(acknowledge_configuration))
        .layer(DefaultBodyLimit::max(512 * 1024))
}

#[utoipa::path(get, path = "/api/v1/networks/{network_id}/policy", operation_id = "getPolicy", security(("adminBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Saved source policy and current diagnostics", body = PolicySnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_policy(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<PolicySnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.select_network(&network_id)?;
            store.policy().map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/policy/preview", operation_id = "previewPolicy", security(("adminBearer" = [])), request_body = PreviewPolicyRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Pure preview; no persistence", body = PreviewPolicyResponse),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn preview_policy(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<PreviewPolicyRequest>, JsonRejection>,
) -> Result<Json<PreviewPolicyResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.select_network(&network_id)?;
            store.preview(&request).map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/policy/save", operation_id = "savePolicy", security(("adminBearer" = [])), request_body = SavePolicyRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Canonical conditional policy save", body = PolicySnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid policy or capacity exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn save_policy(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SavePolicyRequest>, JsonRejection>,
) -> Result<Json<PolicySnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.select_network(&network_id)?;
            store.save_policy(&request).map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/nodes/country", operation_id = "setNodeCountry", security(("adminBearer" = [])), request_body = SetNodeCountryRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Normalized administrator-managed country metadata", body = RevokeNodeResponse),
    (status = 400, description = "Country must be two ASCII letters or null", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 404, description = "Node not found", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Capacity exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn set_node_country(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SetNodeCountryRequest>, JsonRejection>,
) -> Result<Json<RevokeNodeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.select_network(&network_id)?;
            Ok(Json(RevokeNodeResponse {
                node: store.set_country(&request)?,
            }))
        })
        .await
}

#[utoipa::path(get, path = "/api/v1/agents/configuration", operation_id = "getAgentConfiguration", security(("nodeBearer" = [])),
    responses((status = 200, description = "Credential-scoped configuration and active public peers", body = ConfigurationSnapshot),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_agent_configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigurationSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.configuration(&token).map(Json))
        .await
}

#[utoipa::path(post, path = "/api/v1/agents/configuration/ack", operation_id = "acknowledgeConfiguration", security(("nodeBearer" = [])), request_body = AcknowledgeConfigurationRequest,
    responses((status = 200, description = "Validated and durably saved receipt; not networking applied", body = AcknowledgeConfigurationResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Revision or digest is not this node's current snapshot", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn acknowledge_configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AcknowledgeConfigurationRequest>, JsonRejection>,
) -> Result<Json<AcknowledgeConfigurationResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.acknowledge(&token, &request).map(Json))
        .await
}
