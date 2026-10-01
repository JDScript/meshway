use axum::{
    Json,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    AgentDataplaneSnapshot, ErrorResponse, MeshAccessSnapshot, SaveMeshAccessRequest,
};
use utoipa_axum::router::OpenApiRouter;

use crate::{AppState, Error, bearer, json};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_mesh_access, save_mesh_access))
        .routes(utoipa_axum::routes!(get_agent_dataplane))
        .layer(DefaultBodyLimit::max(256 * 1024))
}

#[utoipa::path(get, path = "/api/v1/access", operation_id = "getMeshAccess", security(("adminBearer" = [])),
    responses((status = 200, description = "Saved directional full-path IPv4 authorization and current diagnostics", body = MeshAccessSnapshot),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_mesh_access(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeshAccessSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.mesh_access().map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/access", operation_id = "saveMeshAccess", security(("adminBearer" = [])), request_body = SaveMeshAccessRequest,
    responses((status = 200, description = "Global-CAS authorization save; all participants require bound public keys; adjacency need not exist", body = MeshAccessSnapshot),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid access, missing identity, or capacity exhausted (128 KiB source, 128 grants, 2..8 nodes per path, 64 projected union identities)", body = ErrorResponse),
    (status = 413, description = "Request exceeds 256 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn save_mesh_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<SaveMeshAccessRequest>, JsonRejection>,
) -> Result<Json<MeshAccessSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.save_mesh_access(&request).map(Json)
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
