use super::{auth::bearer, error::HttpError};
use crate::error::Error;
use crate::execution::Execution;
use axum::extract::rejection::JsonRejection;
use axum::{
    Extension, Json,
    extract::{Path, State, rejection::PathRejection},
    http::HeaderMap,
};
use meshway_model::{NetworkId, NodeId, RequestId};
use meshway_protocol::{ErrorResponse, node::*};
/// Read non-secret settings for a Node.
///
/// Requires a user session; ordinary members may read only their current User-identity devices.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes/{nodeId}/settings",operation_id="getNodeSettings",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),responses((status=200,body=NodeSettingsResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn settings(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
) -> Result<Json<NodeSettingsResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let (revision, settings) = service
        .settings(token, network, node)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(NodeSettingsResponse {
        request_id: id,
        network_id: network,
        node_id: node,
        revision,
        settings,
    }))
}
/// Permanently delete a Node and revoke its enrollment identity.
///
/// Requires a network administrator's user session and the expected configuration revision.
/// The request identifier deduplicates retries of the same deletion.
#[utoipa::path(delete,path="/api/v1/networks/{networkId}/nodes/{nodeId}",operation_id="deleteNode",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=DeleteNodeRequest,responses((status=200,body=meshway_protocol::configuration::ConfigurationAppliedResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="revision_conflict, request_id_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
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
        .map_err(|e| HttpError::domain(id, e))
}
/// Permanently revoke an exact Node key binding.
///
/// Requires a network administrator's user session; the expected generation guards against key replacement.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/nodes/{nodeId}/keys/revoke",operation_id="revokeNodeKey",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=RevokeNodeKeyRequest,responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=409,body=ErrorResponse,description="version_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
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
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
/// List visible registered Nodes and their current action eligibility.
///
/// Requires a user session and evaluates visibility within the requested network.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes",operation_id="listNodes",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=NodeInventoryResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
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
        .map_err(|e| HttpError::domain(id, e))
}
/// Read a visible Node's registration and configured identity.
///
/// Requires a user session; an absent or non-visible Node returns 404.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes/{nodeId}",operation_id="getNode",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),responses((status=200,body=NodeInventoryView),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
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
        .map_err(|e| HttpError::domain(id, e))?
        .items
        .into_iter()
        .find(|n| n.id == node)
        .map(Json)
        .ok_or_else(|| HttpError::domain(id, Error::NotFound))
}
/// Read a Node's routing profile selection.
///
/// Requires a user session. Owners and Admins may read any Node; a member only their own User-identity devices, every other Node returns 404.
/// The answer is the requested and applied profile, the allowed profiles with a summary of each, the assignment's lock, how `auto` resolves, and whether the caller may select.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection",operation_id="getNodeRoutingSelection",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),responses((status=200,body=NodeRoutingSelectionResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn routing_selection(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
) -> Result<Json<NodeRoutingSelectionResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let selection = service
        .routing_selection(token, network, node)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(selection_response(id, network, node, selection)))
}
/// Select a Node's routing profile.
///
/// Requires a user session with the same visibility as `getNodeRoutingSelection`: a member selects for their own User-identity devices, Owners and Admins for any Node, Tag Nodes included; the activity records the acting user.
/// The profile must be allowed for the Node and, while its assignment is locked, be the default; otherwise routing_profile_unavailable lists the allowed profiles.
/// The request identifier makes retries of the same selection return the current selection without a second change; reuse with another Node or profile returns request_id_conflict.
/// Acceptance persists the selection as the Node's usage state, recompiles its netmap (ending flows through the previous exit) and records a `routing.select` activity.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection",operation_id="setNodeRoutingSelection",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=SelectNodeRoutingProfileRequest,responses((status=200,body=NodeRoutingSelectionResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="request_id_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="routing_profile_unavailable"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn select_routing_profile(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
    body: Result<Json<SelectNodeRoutingProfileRequest>, JsonRejection>,
) -> Result<Json<NodeRoutingSelectionResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let selection = service
        .select_user_profile(token, network, node, body)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(selection_response(id, network, node, selection)))
}
/// Clear a Node's routing profile selection.
///
/// Requires a user session with the same visibility as `getNodeRoutingSelection`: a member clears for their own User-identity devices, Owners and Admins for any Node, Tag Nodes included; the activity records the acting user.
/// Afterwards the assignment's default applies (`auto` included); a lock does not refuse clearing, since the default is what a lock pins.
/// The request identifier makes retries return the current selection without a second change; reuse with another Node or a selection returns request_id_conflict. Clearing a Node without a selected profile changes nothing; its request is still recorded as a `routing.select` activity whose target profile is null, so that a retry never clears a later selection.
/// Acceptance keeps the version counter, recompiles the Node's netmap (ending flows through the previous exit) and records a `routing.select` activity whose target profile is null.
#[utoipa::path(delete,path="/api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection",operation_id="clearNodeRoutingSelection",tag="nodes",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("nodeId"=NodeId,Path)),request_body=ClearNodeRoutingSelectionRequest,responses((status=200,body=NodeRoutingSelectionResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="request_id_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn clear_routing_selection(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, NodeId)>, PathRejection>,
    body: Result<Json<ClearNodeRoutingSelectionRequest>, JsonRejection>,
) -> Result<Json<NodeRoutingSelectionResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, node)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let selection = service
        .clear_user_profile(token, network, node, body)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(selection_response(id, network, node, selection)))
}
fn selection_response(
    id: RequestId,
    network: NetworkId,
    node: NodeId,
    selection: meshway_configuration::selection::RoutingSelectionState,
) -> NodeRoutingSelectionResponse {
    NodeRoutingSelectionResponse {
        request_id: id,
        network_id: network,
        node_id: node,
        can_select: crate::store::execution::can_select(&selection),
        selection,
    }
}
