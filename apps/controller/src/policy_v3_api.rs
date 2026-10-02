use crate::{AppState, Error, bearer, json};
use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_policy::v3::{Diagnostic, NetworkPolicy};
use meshway_protocol::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveNetworkPolicyRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    /// Explicit replacement/activation; not an assertion of equivalent migration.
    pub policy: NetworkPolicy,
    #[serde(default)]
    pub source_jsonc: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicySnapshot {
    #[schema(min_length = 32, max_length = 32, pattern = "^[0-9a-f]{32}$")]
    pub network_id: String,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    pub source_mode: PolicySourceMode,
    #[schema(required = true)]
    pub policy: Option<NetworkPolicy>,
    #[schema(required = true)]
    pub source_jsonc: Option<String>,
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Proposal only, never automatically activated.
    #[schema(required = true)]
    pub migration_proposal: Option<NetworkPolicy>,
}

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_network_policy))
        .routes(utoipa_axum::routes!(save_network_policy))
        .routes(utoipa_axum::routes!(negotiate_mesh_execution))
        .routes(utoipa_axum::routes!(acknowledge_mesh_execution))
        .layer(DefaultBodyLimit::max(512 * 1024))
}

#[utoipa::path(get, path="/api/v1/networks/{network_id}/policy-v3", operation_id="getNetworkPolicy", security(("managementBearer"=[])), params(("network_id"=String, Path)), responses((status=200, description="Saved source or conservative migration proposal", body=NetworkPolicySnapshot),(status=401,description="Unauthorized",body=ErrorResponse),(status=404,description="Network missing",body=ErrorResponse)))]
async fn get_network_policy(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<NetworkPolicySnapshot>, Error> {
    state
        .network_access(
            bearer(&headers)?,
            network_id,
            crate::accounts::NetworkPermission::Read,
            |store, network| store.network_policy(network).map(Json),
        )
        .await
}

#[utoipa::path(post, path="/api/v1/networks/{network_id}/policy-v3/save", operation_id="saveNetworkPolicy", security(("managementBearer"=[])), params(("network_id"=String, Path)), request_body=SaveNetworkPolicyRequest, responses((status=200,description="Explicit source activation and conditional save",body=NetworkPolicySnapshot),(status=401,description="Unauthorized",body=ErrorResponse),(status=409,description="CAS conflict",body=ErrorResponse),(status=422,description="Invalid source or budget",body=ErrorResponse)))]
async fn save_network_policy(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<SaveNetworkPolicyRequest>, JsonRejection>,
) -> Result<Json<NetworkPolicySnapshot>, Error> {
    let request = json(body)?;
    state
        .network_access(
            bearer(&headers)?,
            network_id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network| store.save_network_policy(network, &request).map(Json),
        )
        .await
}

#[utoipa::path(post,path="/api/v1/agents/mesh-execution",operation_id="negotiateMeshExecution",security(("nodeBearer"=[])),request_body=MeshExecutionRequest,responses((status=200,description="Mutation: publish current-epoch execution capability and atomically fetch coherent private node configuration; not a status read or applied proof",body=MeshExecutionResponse),(status=401,description="Unauthorized",body=ErrorResponse),(status=403,description="Revoked",body=ErrorResponse),(status=409,description="Runtime superseded",body=ErrorResponse),(status=422,description="Unsupported execution capability",body=ErrorResponse)))]
async fn negotiate_mesh_execution(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<MeshExecutionRequest>, JsonRejection>,
) -> Result<Json<MeshExecutionResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.mesh_execution(&token, &request).map(Json))
        .await
}

#[utoipa::path(post,path="/api/v1/agents/mesh-execution/ack",operation_id="acknowledgeMeshExecution",security(("nodeBearer"=[])),request_body=AcknowledgeMeshExecutionRequest,responses((status=200,description="Validated durable receipt, not applied",body=AcknowledgeMeshExecutionResponse),(status=401,description="Unauthorized",body=ErrorResponse),(status=403,description="Revoked",body=ErrorResponse),(status=409,description="Runtime or publication changed",body=ErrorResponse)))]
async fn acknowledge_mesh_execution(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AcknowledgeMeshExecutionRequest>, JsonRejection>,
) -> Result<Json<AcknowledgeMeshExecutionResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.acknowledge_mesh_execution(&token, &request).map(Json))
        .await
}
