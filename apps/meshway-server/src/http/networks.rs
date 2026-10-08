use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{
    CreateNetworkRequest, ErrorResponse, NetworkListResponse, NetworkMembersResponse,
    NetworkPageQuery, NetworkResponse, RenameNetworkRequest,
};

use super::{auth::bearer, error::HttpError};
use crate::networks::Networks;

fn page_query(
    id: RequestId,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<NetworkPageQuery, HttpError> {
    query
        .map(|Query(value)| value)
        .map_err(|_| HttpError::invalid_request(id))
}

/// List networks in which the authenticated user has an active membership.
///
/// Returns oldest-first pages with a default limit of 100 and rechecks visibility on each page.
#[utoipa::path(get, path = "/api/v1/networks", operation_id = "listNetworks", tag = "networks",
    security(("userSession" = [])), params(NetworkPageQuery),
    responses((status = 200, body = NetworkListResponse), (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn list(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<NetworkListResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let query = page_query(id, query)?;
    let page = networks
        .list(token, query.limit.unwrap_or(100), query.cursor)
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok(Json(NetworkListResponse {
        request_id: id,
        items: page.items,
        next_cursor: page.next_cursor,
        as_of: page.as_of,
        total: page.total,
    }))
}

/// Create an isolated network and an Owner membership for the caller.
///
/// Any valid user session may call this; requestId deduplicates matching input and conflicting reuse returns 409.
#[utoipa::path(post, path = "/api/v1/networks", operation_id = "createNetwork", tag = "networks",
    security(("userSession" = [])), request_body = CreateNetworkRequest,
    responses((status = 201, body = NetworkResponse), (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse), (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn create(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<CreateNetworkRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<NetworkResponse>), HttpError> {
    let token = bearer(&headers, id)?;
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let network = networks
        .create(token, body.name, body.request_id)
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok((
        StatusCode::CREATED,
        Json(NetworkResponse {
            request_id: id,
            network,
        }),
    ))
}

/// Read network metadata and the caller's current membership permissions.
///
/// Requires an active membership in the requested network.
#[utoipa::path(get, path = "/api/v1/networks/{networkId}", operation_id = "getNetwork", tag = "networks",
    security(("userSession" = [])), params(("networkId" = NetworkId, Path)),
    responses((status = 200, body = NetworkResponse), (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn get(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, axum::extract::rejection::PathRejection>,
) -> Result<Json<NetworkResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network_id) = path.map_err(|_| HttpError::invalid_request(id))?;
    let network = networks
        .get(token, network_id)
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok(Json(NetworkResponse {
        request_id: id,
        network,
    }))
}

/// Rename a network using its previously read name as a concurrency guard.
///
/// Requires Owner or Admin permission; requestId deduplicates matching rename input.
#[utoipa::path(post, path = "/api/v1/networks/{networkId}/rename", operation_id = "renameNetwork", tag = "networks",
    security(("userSession" = [])), params(("networkId" = NetworkId, Path)), request_body = RenameNetworkRequest,
    responses((status = 200, body = NetworkResponse), (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse),
        (status = 413, body = ErrorResponse), (status = 415, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn rename(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<RenameNetworkRequest>, JsonRejection>,
) -> Result<Json<NetworkResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network_id) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
    let network = networks
        .rename(
            token,
            network_id,
            body.name,
            body.expected_name,
            body.request_id,
        )
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok(Json(NetworkResponse {
        request_id: id,
        network,
    }))
}

/// List current members of a network.
///
/// Requires Owner or Admin permission and returns oldest-first pages with a default limit of 100.
#[utoipa::path(get, path = "/api/v1/networks/{networkId}/members", operation_id = "listNetworkMembers", tag = "networks",
    security(("userSession" = [])), params(("networkId" = NetworkId, Path), NetworkPageQuery),
    responses((status = 200, body = NetworkMembersResponse), (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub(super) async fn members(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, axum::extract::rejection::PathRejection>,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<NetworkMembersResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network_id) = path.map_err(|_| HttpError::invalid_request(id))?;
    let query = page_query(id, query)?;
    let page = networks
        .members(token, network_id, query.limit.unwrap_or(100), query.cursor)
        .await
        .map_err(|error| HttpError::domain(id, error))?;
    Ok(Json(NetworkMembersResponse {
        request_id: id,
        items: page.items,
        next_cursor: page.next_cursor,
        as_of: page.as_of,
        total: page.total,
    }))
}
