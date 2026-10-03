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
    CreateNetworkRequest, ErrorResponse, MemberSummary, NetworkActions, NetworkListResponse,
    NetworkMembersResponse, NetworkPageQuery, NetworkResponse, NetworkSummary,
    RenameNetworkRequest,
};

use super::{auth::bearer, error::HttpError};
use crate::networks::{NetworkView, Networks};

fn view(network: NetworkView) -> NetworkSummary {
    NetworkSummary {
        id: network.id,
        name: network.name,
        created_at: network.created_at,
        current_revision: network.current_revision,
        membership_id: network.membership_id,
        role: network.role,
        actions: NetworkActions {
            rename: network.actions.rename,
            edit_configuration: network.actions.edit_configuration,
            manage_members: network.actions.manage_members,
        },
    }
}

fn page_query(
    id: RequestId,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<NetworkPageQuery, HttpError> {
    query
        .map(|Query(value)| value)
        .map_err(|_| HttpError::invalid_request(id))
}

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
        .map_err(|error| HttpError::network(id, error))?;
    Ok(Json(NetworkListResponse {
        request_id: id,
        items: page.items.into_iter().map(view).collect(),
        next_cursor: page.next_cursor,
        as_of: page.as_of,
    }))
}

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
        .map_err(|error| HttpError::network(id, error))?;
    Ok((
        StatusCode::CREATED,
        Json(NetworkResponse {
            request_id: id,
            network: view(network),
        }),
    ))
}

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
        .map_err(|error| HttpError::network(id, error))?;
    Ok(Json(NetworkResponse {
        request_id: id,
        network: view(network),
    }))
}

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
        .map_err(|error| HttpError::network(id, error))?;
    Ok(Json(NetworkResponse {
        request_id: id,
        network: view(network),
    }))
}

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
        .map_err(|error| HttpError::network(id, error))?;
    Ok(Json(NetworkMembersResponse {
        request_id: id,
        items: page
            .items
            .into_iter()
            .map(|member| MemberSummary {
                membership_id: member.id,
                user_id: member.user_id,
                email: member.email,
                role: member.role,
                version: member.version,
                joined_at: member.joined_at,
            })
            .collect(),
        next_cursor: page.next_cursor,
        as_of: page.as_of,
    }))
}
