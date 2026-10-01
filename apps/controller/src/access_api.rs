use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    AgentDataplaneSnapshot, ErrorResponse, MeshAccessSnapshot, PreviewFlowRequest,
    PreviewFlowResponse, SaveMeshAccessRequest,
};
use utoipa_axum::router::OpenApiRouter;

use crate::{AppState, Error, bearer, json};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_mesh_access, save_mesh_access))
        .routes(utoipa_axum::routes!(get_agent_dataplane))
        .routes(utoipa_axum::routes!(preview_flow))
        .layer(DefaultBodyLimit::max(512 * 1024))
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/flows/preview", operation_id = "previewFlow", security(("adminBearer" = [])), request_body = PreviewFlowRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Combined saved endpoint authorization and routing preview", body = PreviewFlowResponse),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse)))]
async fn preview_flow(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<PreviewFlowRequest>, JsonRejection>,
) -> Result<Json<PreviewFlowResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            let network_id = store.network_id(&network_id)?;
            store.preview_flow(&network_id, &request).map(Json)
        })
        .await
}

#[utoipa::path(get, path = "/api/v1/networks/{network_id}/access", operation_id = "getMeshAccess", security(("adminBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Saved ordered endpoint authorization source and current diagnostics", body = MeshAccessSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_mesh_access(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<MeshAccessSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            let network_id = store.network_id(&network_id)?;
            store.mesh_access(&network_id).map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/access", operation_id = "saveMeshAccess", security(("adminBearer" = [])), request_body = SaveMeshAccessRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Global-CAS ordered endpoint authorization save; missing keys defer only affected runtime paths", body = MeshAccessSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid selectors or references, JSONC mismatch, or expanded publication capacity exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 512 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn save_mesh_access(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SaveMeshAccessRequest>, JsonRejection>,
) -> Result<Json<MeshAccessSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            let network_id = store.network_id(&network_id)?;
            store.save_mesh_access(&network_id, &request).map(Json)
        })
        .await
}

#[utoipa::path(get, path = "/api/v1/agent/dataplane", operation_id = "getAgentDataplane", security(("nodeBearer" = [])),
    responses((status = 200, description = "Atomic node-scoped configuration, adjacency, directional access and public identities. Invalid access projects empty grants. No runtime or ACK assertion.", body = AgentDataplaneSnapshot),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 422, description = "Fails closed above 64 union identities or 2 MiB snapshot", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_agent_dataplane(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentDataplaneSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.agent_dataplane(&token).map(Json))
        .await
}
