use crate::{AppState, Error, bearer, json};
use axum::{
    Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{
    EndpointDiscoveriesSnapshot, ErrorResponse, ReportEndpointDiscoveryRequest,
    ReportEndpointDiscoveryResponse,
};
use utoipa_axum::router::OpenApiRouter;

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(report_endpoint_discovery))
        .routes(utoipa_axum::routes!(get_endpoint_discoveries))
        .layer(DefaultBodyLimit::max(256 * 1024))
}

#[utoipa::path(post, path = "/api/v1/agent/connections/discovery", operation_id = "reportEndpointDiscovery", security(("nodeBearer" = [])), request_body = ReportEndpointDiscoveryRequest,
    responses((status = 200, description = "Leased node-authenticated UDP self-observation; never authorization or peer identity proof", body = ReportEndpointDiscoveryResponse),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Node credential required", body = ErrorResponse),
    (status = 403, description = "Node revoked", body = ErrorResponse),
    (status = 409, description = "Superseded epoch, sequence, generation or socket observation", body = ErrorResponse),
    (status = 422, description = "Invalid configuration scope or discovery claim", body = ErrorResponse),
    (status = 413, description = "Request exceeds 256 KiB", body = ErrorResponse),
    (status = 415, description = "Expected application/json", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn report_endpoint_discovery(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ReportEndpointDiscoveryRequest>, JsonRejection>,
) -> Result<Json<ReportEndpointDiscoveryResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| store.report_endpoint_discovery(&token, &request).map(Json))
        .await
}

#[utoipa::path(get, path = "/api/v1/networks/{network_id}/connections/discovery", operation_id = "getEndpointDiscoveries", security(("adminBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Current leased UDP self-observations; missing reports explicitly await reporting", body = EndpointDiscoveriesSnapshot),
    (status = 404, description = "Network not found", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 500, description = "Storage unavailable", body = ErrorResponse)))]
async fn get_endpoint_discoveries(
    State(state): State<AppState>,
    Path(network_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<EndpointDiscoveriesSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            let network_id = store.network_id(&network_id)?;
            store.endpoint_discoveries(&network_id).map(Json)
        })
        .await
}
