use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse, ConnectionRuntimesSnapshot,
    ErrorResponse, GetConnectionRuntimeEpochResponse, ReportConnectionRuntimeRequest,
    ReportConnectionRuntimeResponse,
};
use utoipa_axum::router::OpenApiRouter;

use super::{bearer, json};
use crate::{AppState, Error};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_connection_runtime_epoch))
        .routes(utoipa_axum::routes!(begin_connection_runtime))
        .layer(DefaultBodyLimit::max(8192))
        .merge(
            OpenApiRouter::new()
                .routes(utoipa_axum::routes!(report_connection_runtime))
                .routes(utoipa_axum::routes!(get_connection_runtimes))
                .layer(DefaultBodyLimit::max(128 * 1024)),
        )
}

#[utoipa::path(get, path = "/api/v1/agent/connections/runtime", operation_id = "getConnectionRuntimeEpoch", security(("nodeBearer" = [])),
    responses((status = 200, description = "Read node-scoped acquisition CAS epoch; zero means no lease; no renewal or mutation", body = GetConnectionRuntimeEpochResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 422, description = "Connection identity must be bound", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_connection_runtime_epoch(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<GetConnectionRuntimeEpochResponse>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| store.get_connection_runtime_epoch(&token).map(Json))
        .await
}

#[utoipa::path(post, path = "/api/v1/agent/connections/runtime/begin", operation_id = "beginConnectionRuntime", security(("nodeBearer" = [])), request_body = BeginConnectionRuntimeRequest,
    responses((status = 200, description = "Acquire runtime epoch by CAS; current instance retries preserve observation freshness", body = BeginConnectionRuntimeResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Expected epoch no longer current for a different instance; lease/report unchanged", body = ErrorResponse),
    (status = 422, description = "Invalid instance ID, missing bound identity or epoch exhausted", body = ErrorResponse),
    (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn begin_connection_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<BeginConnectionRuntimeRequest>, JsonRejection>,
) -> Result<Json<BeginConnectionRuntimeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.begin_connection_runtime(&token, &request).map(Json))
        .await
}

#[utoipa::path(post, path = "/api/v1/agent/connections/runtime", operation_id = "reportConnectionRuntime", security(("nodeBearer" = [])), request_body = ReportConnectionRuntimeRequest,
    responses((status = 200, description = "Record scoped runtime self-report; duplicate sequence preserves server receive time; never ACK or independent peer proof", body = ReportConnectionRuntimeResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Superseded or absent runtime lease, old sequence or changed same-sequence report", body = ErrorResponse),
    (status = 422, description = "Configuration revision/digest mismatch, missing identity or invalid observation scope/state", body = ErrorResponse),
    (status = 413, description = "Request exceeds 128 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn report_connection_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ReportConnectionRuntimeRequest>, JsonRejection>,
) -> Result<Json<ReportConnectionRuntimeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.report_connection_runtime(&token, &request).map(Json))
        .await
}

#[utoipa::path(get, path = "/api/v1/networks/{network_id}/connections/runtime", operation_id = "getConnectionRuntimes", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Leased node self-reports with server receive TTL and current configuration matching; not authorization", body = ConnectionRuntimesSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_connection_runtimes(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ConnectionRuntimesSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Read,
            move |store, network_id| store.connection_runtimes(network_id).map(Json),
        )
        .await
}
