use super::TypedPath as Path;
use super::{
    error::HttpError,
    state::{node, user},
};
use crate::observations::service::{ObservationOperation, ObservationProvider, ObservationService};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
};
use meshway_model::{NetworkId, ObservationId, ObservationSubscriptionId, RequestId};
use meshway_protocol::{ErrorResponse, observations::*};
use meshway_protocol::{InstanceRequest, node_observations::*};

pub fn router<P: ObservationProvider>(service: ObservationService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_list, _, _, _>(Router::new(), list::<P>);
    let router = declared_route::<__path_create, _, _, _>(router, create::<P>);
    let router = declared_route::<__path_detail, _, _, _>(router, detail::<P>);
    let router = declared_route::<__path_renew, _, _, _>(router, renew::<P>);
    let router = declared_route::<__path_cancel, _, _, _>(router, cancel::<P>);
    let router = declared_route::<__path_tasks, _, _, _>(router, tasks::<P>);
    let router = declared_route::<__path_node_create, _, _, _>(router, node_create::<P>);
    let router = declared_route::<__path_node_detail, _, _, _>(router, node_detail::<P>);
    let router = declared_route::<__path_node_renew, _, _, _>(router, node_renew::<P>);
    let router = declared_route::<__path_node_cancel, _, _, _>(router, node_cancel::<P>);
    declared_route::<__path_result, _, _, _>(router, result::<P>)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
/// Create a short-lived diagnostic observation subscription.
///
/// Requires a user session and authority over the source Node and exact service path.
/// Retrying the same subscription identifier does not extend its lease.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations",operation_id="createObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=CreateObservation,responses((status=201,body=ObservationTask),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=429,body=ErrorResponse,description="observation_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn create<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
    body: Result<Json<CreateObservation>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<ObservationTask>), HttpError> {
    let identity = user(&headers, network, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok((
        StatusCode::CREATED,
        Json(
            service
                .create_observation(identity, body)
                .await
                .map_err(|e| HttpError::domain(id, e))?,
        ),
    ))
}
/// List the user session's visible observation subscriptions.
///
/// Requires a user session; each subscription retains its own authorization and lease.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/observations",operation_id="listObservations",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=ObservationList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn list<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
) -> Result<Json<ObservationList>, HttpError> {
    Ok(Json(
        service
            .observations(
                user(&headers, network, id)?,
                None,
                ObservationOperation::Read,
            )
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Read an observation subscription's current result and lifecycle.
///
/// Requires the subscribing user session and current permissions; reading does not renew the lease.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/observations/{subscriptionId}",operation_id="getObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("subscriptionId"=ObservationSubscriptionId,Path)),responses((status=200,body=ObservationList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn detail<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationSubscriptionId)>,
    headers: HeaderMap,
) -> Result<Json<ObservationList>, HttpError> {
    Ok(Json(
        service
            .observations(
                user(&headers, network, id)?,
                Some(observation),
                ObservationOperation::Read,
            )
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Renew an observation subscription under current permissions.
///
/// Requires the subscribing user session; renewal affects only the specified subscription's short lease.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations/{subscriptionId}/renew",operation_id="renewObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("subscriptionId"=ObservationSubscriptionId,Path)),responses((status=200,body=ObservationList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=429,body=ErrorResponse,description="observation_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn renew<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationSubscriptionId)>,
    headers: HeaderMap,
) -> Result<Json<ObservationList>, HttpError> {
    Ok(Json(
        service
            .observations(
                user(&headers, network, id)?,
                Some(observation),
                ObservationOperation::Renew,
            )
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Cancel an observation subscription.
///
/// Requires the subscribing user session; other subscriptions sharing the measurement task remain independent.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations/{subscriptionId}/cancel",operation_id="cancelObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("subscriptionId"=ObservationSubscriptionId,Path)),responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn cancel<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationSubscriptionId)>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    service
        .observations(
            user(&headers, network, id)?,
            Some(observation),
            ObservationOperation::Cancel,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
/// Read diagnostic tasks assigned to the authenticated Node.
///
/// Requires a Node session; tasks remain bound to the source instance, execution target and deadline.
#[utoipa::path(get,path="/agent-api/v1/tasks",operation_id="getNodeTasks",tag="observations",security(("nodeSession"=[])),responses((status=200,body=ObservationList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn tasks<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<ObservationList>, HttpError> {
    Ok(Json(
        service
            .node_tasks(node(&headers, id)?)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Report a diagnostic task's actual phase or measurement window.
///
/// Requires the source Node's session and matching live instance and execution target.
/// Results do not renew task authorization.
#[utoipa::path(post,path="/agent-api/v1/tasks/{taskId}/results",operation_id="reportTaskResult",tag="observations",security(("nodeSession"=[])),params(("taskId"=ObservationId,Path)),request_body=ObservationResult,responses((status=200,body=ObservationTask),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=429,body=ErrorResponse,description="observation_limit, state_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn result<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(task): Path<ObservationId>,
    headers: HeaderMap,
    body: Result<Json<ObservationResult>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ObservationTask>, HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .observation_result(authorization, task, body)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Create an instance-bound Node diagnostic subscription.
///
/// Requires a Node session and current instance qualification for the exact authorized diagnostic scope.
#[utoipa::path(post,path="/agent-api/v1/observations",operation_id="createNodeObservation",tag="observations",security(("nodeSession"=[])),request_body=CreateNodeObservation,responses((status=201,body=NodeObservationResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=429,body=ErrorResponse,description="observation_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn node_create<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<CreateNodeObservation>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<NodeObservationResponse>), HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok((
        StatusCode::CREATED,
        Json(
            service
                .create_node_observation(authorization, body)
                .await
                .map_err(|e| HttpError::domain(id, e))?,
        ),
    ))
}
/// URI values are text. The protocol object's JSON-strict scalar adapter must
/// not be used with serde_urlencoded: it intentionally rejects string numbers.
/// This adapter ends at the HTTP boundary; authorization uses the formal types.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NodeObservationUriQuery {
    instance_id: String,
    generation: String,
}
impl NodeObservationUriQuery {
    fn decode(self) -> Result<NodeObservationQuery, crate::error::Error> {
        let invalid = || crate::error::Error::InvalidRequest;
        if self.generation.is_empty() || !self.generation.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid());
        }
        Ok(NodeObservationQuery {
            instance_id: self.instance_id.parse().map_err(|_| invalid())?,
            generation: meshway_model::Version::new(
                self.generation.parse().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        })
    }
}

/// Read a Node diagnostic subscription.
///
/// Requires the subscribing Node's session and exact instance generation; reading does not extend its lease.
#[utoipa::path(get,path="/agent-api/v1/observations/{subscriptionId}",operation_id="getNodeObservation",tag="observations",security(("nodeSession"=[])),params(("subscriptionId"=ObservationSubscriptionId,Path),("instanceId"=meshway_model::InstanceId,Query),("generation"=meshway_model::Version,Query)),responses((status=200,body=NodeObservationResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn node_detail<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(subscription): Path<ObservationSubscriptionId>,
    headers: HeaderMap,
    query: Result<Query<NodeObservationUriQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<NodeObservationResponse>, HttpError> {
    let authorization = node(&headers, id)?;
    let Query(query) =
        query.map_err(|_| HttpError::domain(id, crate::error::Error::InvalidRequest))?;
    let query = query.decode().map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(
        service
            .node_observation(
                authorization,
                InstanceRequest {
                    instance_id: query.instance_id,
                    generation: query.generation,
                },
                subscription,
                None,
            )
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Renew a Node diagnostic subscription to an absolute requested deadline.
///
/// Requires the subscribing Node's session and current instance; replay does not add a relative duration.
#[utoipa::path(post,path="/agent-api/v1/observations/{subscriptionId}/renew",operation_id="renewNodeObservation",tag="observations",security(("nodeSession"=[])),params(("subscriptionId"=ObservationSubscriptionId,Path)),request_body=RenewNodeObservation,responses((status=200,body=NodeObservationResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=429,body=ErrorResponse,description="observation_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn node_renew<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(subscription): Path<ObservationSubscriptionId>,
    headers: HeaderMap,
    body: Result<Json<RenewNodeObservation>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<NodeObservationResponse>, HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .node_observation(
                authorization,
                body.instance,
                subscription,
                Some(body.expires_at_ms),
            )
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Cancel an instance-bound Node diagnostic subscription.
///
/// Requires the subscribing Node's session and exact instance qualification.
#[utoipa::path(post,path="/agent-api/v1/observations/{subscriptionId}/cancel",operation_id="cancelNodeObservation",tag="observations",security(("nodeSession"=[])),params(("subscriptionId"=ObservationSubscriptionId,Path)),request_body=InstanceRequest,responses((status=200,body=NodeObservationCancelled),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn node_cancel<P: ObservationProvider>(
    State(service): State<ObservationService<P>>,
    Extension(id): Extension<RequestId>,
    Path(subscription): Path<ObservationSubscriptionId>,
    headers: HeaderMap,
    body: Result<Json<InstanceRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<NodeObservationCancelled>, HttpError> {
    let authorization = node(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .cancel_node_observation(authorization, body, subscription)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        create,
        list,
        detail,
        renew,
        cancel,
        tasks,
        result,
        node_create,
        node_detail,
        node_renew,
        node_cancel
    ),
    components(schemas(NodeObservationQuery))
)]
pub struct ObservationApi;
