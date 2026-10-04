//! Actual State/SSE routes with separate transactional user and Node adapters.
//! Execution authenticates Node sessions, instances and publication lifetimes.
use super::TypedPath as Path;
use super::{auth::bearer, error::HttpError};
use crate::state::service::{ReadIdentity, StateProvider, StateService};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    middleware,
    response::{IntoResponse, Response},
};
use meshway_model::{NetworkId, NodeId, RequestId};
use meshway_protocol::{ErrorResponse, observations::*, state::*};
use std::sync::Arc;

pub fn user_router<P: StateProvider>(service: StateService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_network_state, _, _, _>(Router::new(), network_state::<P>);
    let router = declared_route::<__path_node_state, _, _, _>(router, node_state::<P>);
    let router = declared_route::<__path_user_events, _, _, _>(router, user_events::<P>);
    let router = declared_route::<__path_history, _, _, _>(router, history::<P>);
    router
        .layer(DefaultBodyLimit::max(crate::state::MAX_REPORT_BYTES))
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
pub fn node_router<P: StateProvider>(service: StateService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_report, _, _, _>(Router::new(), report::<P>);
    let router = declared_route::<__path_peer_query, _, _, _>(router, peer_query::<P>);
    let router = declared_route::<__path_report_history, _, _, _>(router, report_history::<P>);
    declared_route::<__path_node_events, _, _, _>(router, node_events::<P>)
        .layer(DefaultBodyLimit::max(crate::state::MAX_REPORT_BYTES))
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
pub(super) fn user(
    headers: &HeaderMap,
    network: NetworkId,
    id: RequestId,
) -> Result<ReadIdentity, HttpError> {
    ReadIdentity::user_session(bearer(headers, id)?, network)
        .map_err(|error| HttpError::domain(id, error))
}
pub(super) fn node(headers: &HeaderMap, id: RequestId) -> Result<String, HttpError> {
    let value = bearer(headers, id)?;
    if value.len() > 8192 {
        return Err(HttpError::invalid_request(id));
    }
    Ok(value.to_owned())
}
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/state", operation_id = "getNetworkState", tag = "state", security(("userSession"=[])), params(("networkId"=NetworkId, Path)), responses((status=200, body=StateView),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn network_state<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
) -> Result<Json<StateView>, HttpError> {
    Ok(Json(
        service
            .query(user(&headers, network, id)?, None)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/nodes/{nodeId}/state", operation_id = "getNodeState", tag = "state", security(("userSession"=[])), params(("networkId"=NetworkId, Path),("nodeId"=NodeId, Path)), responses((status=200, body=StateView),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn node_state<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, node_id)): Path<(NetworkId, NodeId)>,
    headers: HeaderMap,
) -> Result<Json<StateView>, HttpError> {
    Ok(Json(
        service
            .query(user(&headers, network, id)?, Some(vec![node_id]))
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/state", operation_id = "reportNodeState", tag = "state", security(("nodeSession"=[])), request_body=StateReport, responses((status=200, body=StateAccepted),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn report<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<StateReport>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<StateAccepted>, HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .accept_report(authorization, body)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/state/query", operation_id = "queryPeerState", tag = "state", security(("nodeSession"=[])), request_body=StateQuery, responses((status=200, body=StateView),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn peer_query<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<StateQuery>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<StateView>, HttpError> {
    let identity = ReadIdentity::Node {
        authorization: node(&headers, id)?,
    };
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .query(identity, body.nodes)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/history", operation_id = "reportNodeHistory", tag = "observations", security(("nodeSession"=[])), request_body=HistoryReport, responses((status=200, body=HistoryAccepted),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn report_history<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<HistoryReport>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<HistoryAccepted>, HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .accept_history(authorization, body)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post, path = "/api/v1/networks/{networkId}/history/query", operation_id = "queryObservationHistory", tag = "observations", security(("userSession"=[])), params(("networkId"=NetworkId, Path)), request_body=HistoryQuery, responses((status=200, body=HistoryPage),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn history<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
    body: Result<Json<HistoryQuery>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<HistoryPage>, HttpError> {
    let identity = user(&headers, network, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .history(identity, body)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
fn last_cursor(headers: &HeaderMap, id: RequestId) -> Result<Option<String>, HttpError> {
    if headers.get_all("last-event-id").iter().count() > 1 {
        return Err(HttpError::invalid_request(id));
    }
    headers
        .get("last-event-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .filter(|v| !v.is_empty() && v.len() <= 4096)
                .map(str::to_owned)
                .ok_or_else(|| HttpError::invalid_request(id))
        })
        .transpose()
}
async fn events<P: StateProvider>(
    service: StateService<P>,
    identity: ReadIdentity,
    headers: HeaderMap,
    id: RequestId,
) -> Result<Response, HttpError> {
    let subscription = service
        .open_stream(identity, last_cursor(&headers, id)?, id)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    let mut response =
        crate::state::stream::response(Arc::new(service), subscription).into_response();
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/events", operation_id = "watchNetwork", tag = "events", security(("userSession"=[])), params(("networkId"=NetworkId, Path), ("Last-Event-ID"=Option<String>, Header, nullable=false)), responses((status=200, content_type="text/event-stream", body=String),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn user_events<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    events(service, user(&headers, network, id)?, headers, id).await
}
#[utoipa::path(get, path = "/agent-api/v1/events", operation_id = "watchNode", tag = "events", security(("nodeSession"=[])), params(("Last-Event-ID"=Option<String>, Header, nullable=false)), responses((status=200, content_type="text/event-stream", body=String),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn node_events<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    events(
        service,
        ReadIdentity::Node {
            authorization: node(&headers, id)?,
        },
        headers,
        id,
    )
    .await
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    network_state,
    node_state,
    report,
    peer_query,
    report_history,
    history,
    user_events,
    node_events
))]
pub struct StateApi;

pub(crate) fn progress_router(
    service: StateService<crate::state::provider::UserProvider>,
) -> Router {
    super::declared_route::<__path_progress, _, _, _>(Router::new(), progress)
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
#[utoipa::path(get, path="/api/v1/networks/{networkId}/configuration/revisions/{revision}/progress", operation_id="getConfigurationProgress", tag="configuration", security(("userSession"=[])), params(("networkId"=NetworkId, Path),("revision"=meshway_model::Revision, Path)), responses((status=200, body=meshway_protocol::ConfigurationApplyProgress),(status=400, body=ErrorResponse),(status=401, body=ErrorResponse),(status=403, body=ErrorResponse),(status=404, body=ErrorResponse),(status=409, body=ErrorResponse),(status=429, body=ErrorResponse),(status=503, body=ErrorResponse)))]
async fn progress(
    State(service): State<StateService<crate::state::provider::UserProvider>>,
    Extension(id): Extension<RequestId>,
    Path((network, revision)): Path<(NetworkId, meshway_model::Revision)>,
    headers: HeaderMap,
) -> Result<Json<meshway_protocol::ConfigurationApplyProgress>, HttpError> {
    let identity = user(&headers, network, id)?;
    let provider = Arc::clone(&service.provider);
    let result = service
        .handle
        .read(move |tx| provider.progress(tx, &identity, revision, chrono::Utc::now()))
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok(Json(result))
}
#[derive(utoipa::OpenApi)]
#[openapi(paths(progress))]
pub struct ProgressApi;
