use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    AcknowledgeAgentConnectionsRequest, AcknowledgeAgentConnectionsResponse,
    AgentConnectionsSnapshot, ConnectionsSnapshot, ErrorResponse, PublishConnectionIdentityRequest,
    PublishConnectionIdentityResponse, SaveConnectionsRequest,
};
use utoipa_axum::router::OpenApiRouter;

use super::{bearer, json};
use crate::{AppState, Error};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(publish_connection_identity))
        .routes(utoipa_axum::routes!(get_agent_connections))
        .routes(utoipa_axum::routes!(acknowledge_agent_connections))
        .layer(DefaultBodyLimit::max(8192))
        .merge(
            OpenApiRouter::new()
                .routes(utoipa_axum::routes!(get_connections, save_connections))
                .layer(DefaultBodyLimit::max(128 * 1024)),
        )
}

#[utoipa::path(post, path = "/api/v1/agent/connection-identity", operation_id = "publishConnectionIdentity", security(("nodeBearer" = [])), request_body = PublishConnectionIdentityRequest,
    responses((status = 200, description = "Immutable node-scoped control-plane public-key association; not runtime peer proof", body = PublishConnectionIdentityResponse),
    (status = 400, description = "Invalid JSON, key or capability advertisement", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Node or public key already differently bound, including revoked tombstones", body = ErrorResponse),
    (status = 422, description = "Revision or publication capacity exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn publish_connection_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<PublishConnectionIdentityRequest>, JsonRejection>,
) -> Result<Json<PublishConnectionIdentityResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store
                .publish_connection_identity(&token, &request)
                .map(Json)
        })
        .await
}

#[utoipa::path(get, path = "/api/v1/networks/{network_id}/connections", operation_id = "getConnections", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Source connection plan, validity and inventory with independent saved-only receipts", body = ConnectionsSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_connections(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ConnectionsSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Read,
            move |store, network_id| store.connections(network_id).map(Json),
        )
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/connections", operation_id = "saveConnections", security(("managementBearer" = [])), request_body = SaveConnectionsRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Canonical global-CAS connection intent save; no runtime activation or transit/exit grant", body = ConnectionsSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid connection plan or capacity exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 128 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn save_connections(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SaveConnectionsRequest>, JsonRejection>,
) -> Result<Json<ConnectionsSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network_id| store.save_connections(network_id, &request).map(Json),
        )
        .await
}

#[utoipa::path(get, path = "/api/v1/agent/connections", operation_id = "getAgentConnections", security(("nodeBearer" = [])),
    responses((status = 200, description = "Node-scoped connection intent with only local profile references and approved adjacent peers", body = AgentConnectionsSnapshot),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_agent_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentConnectionsSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.agent_connections(&token).map(Json))
        .await
}

#[utoipa::path(post, path = "/api/v1/agent/connections/ack", operation_id = "acknowledgeAgentConnections", security(("nodeBearer" = [])), request_body = AcknowledgeAgentConnectionsRequest,
    responses((status = 200, description = "Independently validated and durably saved connection receipt; not applied or connected", body = AcknowledgeAgentConnectionsResponse),
    (status = 400, description = "Invalid JSON, revision or digest", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Revision or digest is not this node's current connections snapshot", body = ErrorResponse),
    (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn acknowledge_agent_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AcknowledgeAgentConnectionsRequest>, JsonRejection>,
) -> Result<Json<AcknowledgeAgentConnectionsResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.acknowledge_connections(&token, &request).map(Json))
        .await
}
