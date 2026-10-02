use super::{bearer, json};
use crate::{AppState, Error};
use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::*;
use utoipa_axum::router::OpenApiRouter;

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(
            get_managed_listeners,
            save_managed_listener
        ))
        .routes(utoipa_axum::routes!(get_agent_managed_connections))
        .routes(utoipa_axum::routes!(get_agent_managed_dataplane))
        .routes(utoipa_axum::routes!(create_managed_enrollment_token))
        .routes(utoipa_axum::routes!(publish_managed_transport_capabilities))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

#[utoipa::path(post,path="/api/v1/agent/managed-transport-capabilities",operation_id="publishManagedTransportCapabilities",security(("nodeBearer"=[])),request_body=PublishManagedTransportCapabilitiesRequest,
 responses((status=200,description="Compiled managed stream capabilities; not listener activation",body=PublishManagedTransportCapabilitiesResponse),(status=400,description="Invalid capability report",body=ErrorResponse),(status=401,description="Node credential required",body=ErrorResponse),(status=403,description="Node revoked",body=ErrorResponse),(status=422,description="Publication capacity exhausted",body=ErrorResponse),(status=413,description="Body exceeds 32 KiB",body=ErrorResponse),(status=415,description="Expected JSON",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn publish_managed_transport_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<PublishManagedTransportCapabilitiesRequest>, JsonRejection>,
) -> Result<Json<PublishManagedTransportCapabilitiesResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store
                .publish_managed_capabilities(&token, &request)
                .map(Json)
        })
        .await
}

#[utoipa::path(post,path="/api/v1/networks/{network_id}/managed-enrollment-tokens",operation_id="createManagedEnrollmentToken",security(("managementBearer"=[])),params(("network_id"=String,Path)),request_body=CreateManagedEnrollmentTokenRequest,
 responses((status=200,description="One-time token bound to initial public listener intent; enrollment binds node atomically",body=CreateEnrollmentTokenResponse),(status=400,description="Invalid JSON or TTL",body=ErrorResponse),(status=401,description="Administrator required",body=ErrorResponse),(status=404,description="Network not found",body=ErrorResponse),(status=422,description="Invalid listener intent",body=ErrorResponse),(status=413,description="Body exceeds 32 KiB",body=ErrorResponse),(status=415,description="Expected JSON",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn create_managed_enrollment_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<CreateManagedEnrollmentTokenRequest>, JsonRejection>,
) -> Result<Json<CreateEnrollmentTokenResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .network_access(
            token,
            id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network| store.create_managed_token(network, &request).map(Json),
        )
        .await
}

#[utoipa::path(get,path="/api/v1/networks/{network_id}/managed-listeners",operation_id="getManagedListeners",security(("managementBearer"=[])),params(("network_id"=String,Path)),
 responses((status=200,description="Public listener intent and material metadata only; not active or reachable",body=ManagedListenersSnapshot),(status=401,description="Administrator required",body=ErrorResponse),(status=404,description="Network not found",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn get_managed_listeners(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ManagedListenersSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .network_access(
            token,
            id,
            crate::accounts::NetworkPermission::Read,
            move |store, network| store.managed_listeners(network).map(Json),
        )
        .await
}

#[utoipa::path(post,path="/api/v1/networks/{network_id}/managed-listeners",operation_id="saveManagedListener",security(("managementBearer"=[])),params(("network_id"=String,Path)),request_body=SaveManagedListenerRequest,
 responses((status=200,description="CAS public settings save; secrets are generated and never returned here",body=ManagedListenersSnapshot),(status=400,description="Invalid JSON",body=ErrorResponse),(status=401,description="Administrator required",body=ErrorResponse),(status=404,description="Network or node not found",body=ErrorResponse),(status=409,description="Stale revision or immutable owner/transport conflict",body=ErrorResponse),(status=422,description="Invalid settings or publication capacity",body=ErrorResponse),(status=413,description="Body exceeds 32 KiB",body=ErrorResponse),(status=415,description="Expected JSON",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn save_managed_listener(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SaveManagedListenerRequest>, JsonRejection>,
) -> Result<Json<ManagedListenersSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .network_access(
            token,
            id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network| store.save_managed_listener(network, &request).map(Json),
        )
        .await
}

#[utoipa::path(get,path="/api/v1/agent/managed-connections",operation_id="getAgentManagedConnections",security(("nodeBearer"=[])),
 responses((status=200,description="Coherent node-only connection/material snapshot; listener keys only to owner",body=AgentManagedConnectionsSnapshot),(status=401,description="Node required",body=ErrorResponse),(status=403,description="Node revoked",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn get_agent_managed_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentManagedConnectionsSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.agent_managed_connections(&token).map(Json))
        .await
}

#[utoipa::path(get,path="/api/v1/agent/managed-dataplane",operation_id="getAgentManagedDataplane",security(("nodeBearer"=[])),
 responses((status=200,description="Coherent policy, public keys, connections and node-only material snapshot",body=AgentManagedDataplaneSnapshot),(status=401,description="Node required",body=ErrorResponse),(status=403,description="Node revoked",body=ErrorResponse),(status=422,description="Publication capacity exhausted",body=ErrorResponse),(status=500,description="Storage unavailable",body=ErrorResponse)))]
async fn get_agent_managed_dataplane(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentManagedDataplaneSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.agent_managed_dataplane(&token).map(Json))
        .await
}
