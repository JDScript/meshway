use super::{auth::bearer, error::HttpError};
use crate::history::History;
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::HeaderMap,
};
use meshway_model::{ActivityId, NetworkId, RequestId};
use meshway_protocol::{ErrorResponse, NetworkPageQuery, history::*};

fn history_error(id: RequestId, error: crate::networks::NetworkError) -> HttpError {
    if error == crate::networks::NetworkError::InvalidCursor {
        HttpError::invalid_request(id)
    } else {
        HttpError::network(id, error)
    }
}

#[utoipa::path(get,path="/api/v1/account/activity",operation_id="listAccountActivity",tag="account",security(("userSession"=[])),params(NetworkPageQuery),responses((status=200,body=ActivityListResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn account(
    State(service): State<History>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<ActivityListResponse>, HttpError> {
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        service
            .activities(
                bearer(&headers, id)?,
                None,
                query.limit.unwrap_or(50),
                query.cursor,
                id,
            )
            .await
            .map_err(|e| history_error(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/activity",operation_id="listNetworkActivity",tag="history",security(("userSession"=[])),params(("networkId"=NetworkId,Path),NetworkPageQuery),responses((status=200,body=ActivityListResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn network(
    State(service): State<History>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<ActivityListResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        service
            .activities(
                bearer(&headers, id)?,
                Some(network),
                query.limit.unwrap_or(50),
                query.cursor,
                id,
            )
            .await
            .map_err(|e| history_error(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/activity/{activityId}",operation_id="getNetworkActivity",tag="history",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("activityId"=ActivityId,Path)),responses((status=200,body=ActivityResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn activity(
    State(service): State<History>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, ActivityId)>, PathRejection>,
) -> Result<Json<ActivityResponse>, HttpError> {
    let Path((network, activity)) = path.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        service
            .activity(bearer(&headers, id)?, network, activity, id)
            .await
            .map_err(|e| history_error(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/configuration/revisions",operation_id="listConfigurationRevisions",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path),NetworkPageQuery),responses((status=200,body=ConfigurationRevisionList),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn revisions(
    State(service): State<History>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<ConfigurationRevisionList>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        service
            .revisions(
                bearer(&headers, id)?,
                network,
                query.limit.unwrap_or(50),
                query.cursor,
                id,
            )
            .await
            .map_err(|e| history_error(id, e))?,
    ))
}
