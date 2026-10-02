use crate::{AppState, Error, bearer, json};
use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    ErrorResponse, LatencyHistoryResponse, QueryLatencyHistoryRequest, ReportLatencySamplesRequest,
    ReportLatencySamplesResponse,
};
use utoipa_axum::router::OpenApiRouter;

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(report_latency_samples))
        .routes(utoipa_axum::routes!(query_latency_history))
        .layer(DefaultBodyLimit::max(128 * 1024))
}

#[utoipa::path(post, path = "/api/v1/agent/latency/samples", operation_id = "reportLatencySamples", security(("nodeBearer" = [])), request_body = ReportLatencySamplesRequest,
    responses((status = 200, description = "Immutable directed probe events; duplicate IDs preserve original receipt time", body = ReportLatencySamplesResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Runtime superseded, stale or inactive, or changed duplicate event", body = ErrorResponse),
    (status = 422, description = "Invalid event scope, clock or budget", body = ErrorResponse),
    (status = 413, description = "Request exceeds 128 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn report_latency_samples(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ReportLatencySamplesRequest>, JsonRejection>,
) -> Result<Json<ReportLatencySamplesResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.report_latency_samples(&token, &request).map(Json))
        .await
}

#[utoipa::path(post, path = "/api/v1/networks/{network_id}/latency/history/query", operation_id = "queryLatencyHistory", security(("adminBearer" = [])), request_body = QueryLatencyHistoryRequest,
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Bounded raw historical events for one directed pair; no synthetic gap or loss points", body = LatencyHistoryResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Read-network credential required", body = ErrorResponse),
    (status = 404, description = "Network or pair not found in network", body = ErrorResponse),
    (status = 422, description = "Window or point budget exceeded", body = ErrorResponse),
    (status = 413, description = "Request exceeds 128 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn query_latency_history(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<QueryLatencyHistoryRequest>, JsonRejection>,
) -> Result<Json<LatencyHistoryResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            let network = store.authorize_latency_history_read(&token, &network_id)?;
            store.latency_history(&network, &request).map(Json)
        })
        .await
}
