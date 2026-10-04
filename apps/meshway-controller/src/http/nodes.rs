use super::{agent::error, auth::bearer, error::HttpError};
use crate::execution::{Execution, NodeError};
use axum::extract::rejection::JsonRejection;
use axum::{
    Extension, Json,
    extract::{Path, State, rejection::PathRejection},
    http::HeaderMap,
};
use meshway_model::{NetworkId, NodeId, PublicationId, RequestId};
use meshway_protocol::{ErrorResponse, execution::PublicationProgress, node::*};
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes/{nodeId}/settings",operation_id="getNodeSettings",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),responses((status=200,body=meshway_configuration::NodeSettingsView),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse)))]
pub(super) async fn settings(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
) -> Result<Json<meshway_configuration::NodeSettingsView>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .settings(token, network, node)
        .await
        .map(Json)
        .map_err(|e| error(id, e))
}
#[utoipa::path(delete,path="/api/v1/networks/{networkId}/nodes/{nodeId}",operation_id="deleteNode",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=DeleteNodeRequest,responses((status=200,body=meshway_protocol::configuration::ConfigurationAppliedResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=409,body=ErrorResponse)))]
pub(super) async fn delete(
    State(service): State<crate::configuration::Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
    body: Result<Json<DeleteNodeRequest>, JsonRejection>,
) -> Result<Json<meshway_protocol::configuration::ConfigurationAppliedResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .apply(
            token,
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: body.expected_revision,
                request_id: body.request_id,
                ops: vec![meshway_configuration::ConfigurationOperation::DeleteNode {
                    node_id: node,
                }],
            },
            id,
        )
        .await
        .map(Json)
        .map_err(|e| HttpError::configuration(id, e))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/nodes/{nodeId}/keys/revoke",operation_id="revokeNodeKey",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=RevokeNodeKeyRequest,responses((status=204),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=409,body=ErrorResponse)))]
pub(super) async fn revoke_key(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
    body: Result<Json<RevokeNodeKeyRequest>, JsonRejection>,
) -> Result<axum::http::StatusCode, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .revoke_key(token, network, node, body)
        .await
        .map_err(|e| error(id, e))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes",operation_id="listNodes",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=NodeInventoryResponse),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse)))]
pub(super) async fn list(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
) -> Result<Json<NodeInventoryResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .inventory(token, network)
        .await
        .map(Json)
        .map_err(|e| error(id, e))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes/{nodeId}",operation_id="getNode",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),responses((status=200,body=NodeInventoryView),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse)))]
pub(super) async fn get(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
) -> Result<Json<NodeInventoryView>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .inventory(token, network)
        .await
        .map_err(|e| error(id, e))?
        .items
        .into_iter()
        .find(|n| n.id == node)
        .map(Json)
        .ok_or_else(|| error(id, NodeError::NotFound))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/publications/{publicationId}",operation_id="getExecutionPublication",tag="execution",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("publicationId"=PublicationId,Path)),responses((status=200,body=PublicationProgress),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse)))]
pub(super) async fn progress(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, PublicationId)>, PathRejection>,
) -> Result<Json<PublicationProgress>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, publication)) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .progress(token, network, publication)
        .await
        .map(Json)
        .map_err(|e| error(id, e))
}
