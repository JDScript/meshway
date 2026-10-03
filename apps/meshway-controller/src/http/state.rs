//! Actual State/SSE routes, composed with the current transactional providers.
//! The base App does not fabricate Node principals when B is not installed.
use super::TypedPath as Path;
use super::{auth::bearer, error::HttpError};
use crate::state::{
    StateError,
    service::{ReadIdentity, StateProvider, StateService},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
};
use meshway_model::{NetworkId, NodeId, RequestId};
use meshway_protocol::{EmptyParams, ErrorResponse, Problem, observations::*, state::*};
use std::sync::Arc;

pub fn router<P: StateProvider>(service: StateService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_network_state, _, _, _>(Router::new(), network_state::<P>);
    let router = declared_route::<__path_node_state, _, _, _>(router, node_state::<P>);
    let router = declared_route::<__path_user_events, _, _, _>(router, user_events::<P>);
    let router = declared_route::<__path_history, _, _, _>(router, history::<P>);
    let router = declared_route::<__path_report, _, _, _>(router, report::<P>);
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
    let digest = crate::accounts::token_digest(bearer(headers, id)?)
        .map_err(|_| HttpError::invalid_session(id))?;
    Ok(ReadIdentity::UserSession { digest, network })
}
pub(super) fn node(headers: &HeaderMap, id: RequestId) -> Result<String, HttpError> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| HttpError::invalid_session(id))?;
    if value.len() > 8192 {
        return Err(HttpError::invalid_request(id));
    }
    Ok(value.to_owned())
}
pub(super) fn failure(id: RequestId, error: StateError) -> HttpError {
    let (status, problem) = match error {
        StateError::Unauthorized => return HttpError::invalid_session(id),
        StateError::Unavailable | StateError::Storage => return HttpError::unavailable(Some(id)),
        StateError::NotFound => (
            StatusCode::NOT_FOUND,
            Problem::NotFound {
                params: EmptyParams {},
                message: "Resource not available.".into(),
            },
        ),
        StateError::Forbidden => (
            StatusCode::FORBIDDEN,
            Problem::PermissionDenied {
                params: EmptyParams {},
                message: "Operation not permitted.".into(),
            },
        ),
        StateError::Conflict => (
            StatusCode::CONFLICT,
            Problem::StateConflict {
                params: EmptyParams {},
                message: "State context or sequence conflicts.".into(),
            },
        ),
        StateError::Cursor => (
            StatusCode::BAD_REQUEST,
            Problem::InvalidCursor {
                params: EmptyParams {},
                message: "Cursor is invalid or expired.".into(),
            },
        ),
        StateError::Budget => (
            StatusCode::TOO_MANY_REQUESTS,
            Problem::StateLimit {
                params: EmptyParams {},
                message: "State resource budget exceeded.".into(),
            },
        ),
        StateError::ObservationLimit => (
            StatusCode::TOO_MANY_REQUESTS,
            Problem::ObservationLimit {
                params: EmptyParams {},
                message: "Observation budget exceeded.".into(),
            },
        ),
        StateError::Invalid => return HttpError::invalid_request(id),
    };
    HttpError::new(id, status, problem)
}

#[utoipa::path(get, path = "/api/v1/networks/{networkId}/state", operation_id = "getNetworkState", tag = "state", security(("userSession"=[])), params(("networkId"=NetworkId, Path)), responses((status=200, body=StateView),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/nodes/{nodeId}/state", operation_id = "getNodeState", tag = "state", security(("userSession"=[])), params(("networkId"=NetworkId, Path),("nodeId"=NodeId, Path)), responses((status=200, body=StateView),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/state", operation_id = "reportNodeState", tag = "state", security(("nodeCredential"=[])), request_body=StateReport, responses((status=200, body=StateAccepted),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/state/query", operation_id = "queryPeerState", tag = "state", security(("nodeCredential"=[])), request_body=StateQuery, responses((status=200, body=StateView),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post, path = "/agent-api/v1/history", operation_id = "reportNodeHistory", tag = "observations", security(("nodeCredential"=[])), request_body=HistoryReport, responses((status=200, body=HistoryAccepted),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post, path = "/api/v1/networks/{networkId}/history/query", operation_id = "queryObservationHistory", tag = "observations", security(("userSession"=[])), params(("networkId"=NetworkId, Path)), request_body=HistoryQuery, responses((status=200, body=HistoryPage),(status=401, body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
fn last_cursor(headers: &HeaderMap, id: RequestId) -> Result<Option<String>, HttpError> {
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
        .open_stream(identity, last_cursor(&headers, id)?)
        .await
        .map_err(|e| failure(id, e))?;
    let mut response =
        crate::state::stream::response(Arc::new(service), subscription).into_response();
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/events", operation_id = "watchNetwork", tag = "events", security(("userSession"=[])), params(("networkId"=NetworkId, Path), ("Last-Event-ID"=Option<String>, Header)), responses((status=200, content_type="text/event-stream", body=String),(status=401, body=ErrorResponse)))]
async fn user_events<P: StateProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    events(service, user(&headers, network, id)?, headers, id).await
}
#[utoipa::path(get, path = "/agent-api/v1/events", operation_id = "watchNode", tag = "events", security(("nodeCredential"=[])), params(("Last-Event-ID"=Option<String>, Header)), responses((status=200, content_type="text/event-stream", body=String),(status=401, body=ErrorResponse)))]
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
