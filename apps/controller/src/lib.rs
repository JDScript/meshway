//! Loopback-first development control plane with route-linked OpenAPI.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::StatusCode,
};
use chrono::Utc;
use meshway_protocol::{
    AgentReport, ErrorResponse, NodeList, NodeSummary, Overview, ReportResponse,
};
use tokio::sync::RwLock;
use utoipa::openapi::{Info, OpenApi};
use utoipa_axum::{router::OpenApiRouter, routes};

const MAX_NODES: usize = 4096;
const MAX_REPORT_BYTES: usize = 8192;

#[derive(Clone, Default)]
pub struct AppState {
    nodes: Arc<RwLock<BTreeMap<String, NodeSummary>>>,
}

/// The same route registrations construct the live router and the contract.
pub fn router_and_openapi(state: AppState) -> (Router, OpenApi) {
    let (router, mut api) = OpenApiRouter::new()
        .routes(routes!(get_overview))
        .routes(routes!(list_nodes))
        .routes(routes!(report_agent))
        .layer(DefaultBodyLimit::max(MAX_REPORT_BYTES))
        .with_state(state)
        .split_for_parts();
    api.info = Info::new("Meshway development control API", env!("CARGO_PKG_VERSION"));
    api.info.description = Some(
        "In-memory development process reporting only. Reports are unauthenticated and are not secure enrollment, mesh membership, connectivity, or tunnel health. State resets on restart."
            .to_owned(),
    );
    (router, api)
}

pub fn router(state: AppState) -> Router {
    router_and_openapi(state).0
}

/// Stable JSON: schema/path maps are ordered and no runtime state is included.
pub fn openapi_json() -> Result<String, serde_json::Error> {
    let api = router_and_openapi(AppState::default()).1;
    Ok(format!("{}\n", serde_json::to_string_pretty(&api)?))
}

#[utoipa::path(
    get, path = "/api/v1/overview", operation_id = "getOverview",
    responses((status = 200, description = "Controller identity and received report count", body = Overview))
)]
async fn get_overview(State(state): State<AppState>) -> Json<Overview> {
    Json(Overview {
        name: "Meshway".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        node_count: state.nodes.read().await.len() as u32,
    })
}

#[utoipa::path(
    get, path = "/api/v1/nodes", operation_id = "listNodes",
    responses((status = 200, description = "Received process reports, sorted by node ID", body = NodeList))
)]
async fn list_nodes(State(state): State<AppState>) -> Json<NodeList> {
    Json(NodeList {
        nodes: state.nodes.read().await.values().cloned().collect(),
    })
}

/// Report a development process. No authentication or secure enrollment exists yet.
#[utoipa::path(
    post, path = "/api/v1/agents/report", operation_id = "reportAgent",
    request_body = AgentReport,
    responses(
        (status = 200, description = "Report stored or updated", body = ReportResponse),
        (status = 400, description = "Invalid JSON or report fields", body = ErrorResponse),
        (status = 413, description = "Report exceeds 8192 bytes", body = ErrorResponse),
        (status = 415, description = "Content-Type must be application/json", body = ErrorResponse),
        (status = 503, description = "Development node capacity reached", body = ErrorResponse)
    )
)]
async fn report_agent(
    State(state): State<AppState>,
    body: Result<Json<AgentReport>, JsonRejection>,
) -> Result<Json<ReportResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Json(report) = body.map_err(|rejection| {
        let (status, message) = match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => {
                (StatusCode::PAYLOAD_TOO_LARGE, "report exceeds 8192 bytes")
            }
            StatusCode::UNSUPPORTED_MEDIA_TYPE => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json",
            ),
            _ => (StatusCode::BAD_REQUEST, "invalid report JSON"),
        };
        api_error(status, message)
    })?;
    report
        .validate()
        .map_err(|message| api_error(StatusCode::BAD_REQUEST, message))?;
    let mut nodes = state.nodes.write().await;
    if nodes.len() >= MAX_NODES && !nodes.contains_key(&report.node_id) {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "development node capacity reached",
        ));
    }
    let node = NodeSummary {
        id: report.node_id,
        name: report.name,
        agent_version: report.agent_version,
        last_seen: Utc::now(),
    };
    nodes.insert(node.id.clone(), node.clone());
    Ok(Json(ReportResponse { node }))
}

fn api_error(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            message: message.to_owned(),
        }),
    )
}
