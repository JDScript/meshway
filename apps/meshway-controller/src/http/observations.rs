use super::TypedPath as Path;
use super::{
    error::HttpError,
    state::{failure, node, user},
};
use crate::{
    observations::service::{ObservationOperation, ObservationProvider},
    state::service::StateService,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    middleware,
};
use meshway_model::{NetworkId, ObservationId, RequestId};
use meshway_protocol::{ErrorResponse, observations::*};

pub fn router<P: ObservationProvider>(service: StateService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_list, _, _, _>(Router::new(), list::<P>);
    let router = declared_route::<__path_create, _, _, _>(router, create::<P>);
    let router = declared_route::<__path_detail, _, _, _>(router, detail::<P>);
    let router = declared_route::<__path_renew, _, _, _>(router, renew::<P>);
    let router = declared_route::<__path_cancel, _, _, _>(router, cancel::<P>);
    let router = declared_route::<__path_tasks, _, _, _>(router, tasks::<P>);
    declared_route::<__path_result, _, _, _>(router, result::<P>)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations",operation_id="createObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=CreateObservation,responses((status=201,body=ObservationTask),(status=401,body=ErrorResponse)))]
async fn create<P: ObservationProvider>(
    State(service): State<StateService<P>>,
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
                .map_err(|e| failure(id, e))?,
        ),
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/observations",operation_id="listObservations",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=ObservationList),(status=401,body=ErrorResponse)))]
async fn list<P: ObservationProvider>(
    State(service): State<StateService<P>>,
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/observations/{observationId}",operation_id="getObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("observationId"=ObservationId,Path)),responses((status=200,body=ObservationList),(status=401,body=ErrorResponse)))]
async fn detail<P: ObservationProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationId)>,
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations/{observationId}/renew",operation_id="renewObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("observationId"=ObservationId,Path)),responses((status=200,body=ObservationList),(status=401,body=ErrorResponse)))]
async fn renew<P: ObservationProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationId)>,
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/observations/{observationId}/cancel",operation_id="cancelObservation",tag="observations",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("observationId"=ObservationId,Path)),responses((status=204),(status=401,body=ErrorResponse)))]
async fn cancel<P: ObservationProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, observation)): Path<(NetworkId, ObservationId)>,
    headers: HeaderMap,
) -> Result<StatusCode, HttpError> {
    service
        .observations(
            user(&headers, network, id)?,
            Some(observation),
            ObservationOperation::Cancel,
        )
        .await
        .map_err(|e| failure(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/agent-api/v1/tasks",operation_id="getNodeTasks",tag="observations",security(("nodeCredential"=[])),responses((status=200,body=ObservationList),(status=401,body=ErrorResponse)))]
async fn tasks<P: ObservationProvider>(
    State(service): State<StateService<P>>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<ObservationList>, HttpError> {
    Ok(Json(
        service
            .node_tasks(node(&headers, id)?)
            .await
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post,path="/agent-api/v1/tasks/{taskId}/results",operation_id="reportTaskResult",tag="observations",security(("nodeCredential"=[])),params(("taskId"=ObservationId,Path)),request_body=ObservationResult,responses((status=200,body=ObservationTask),(status=401,body=ErrorResponse)))]
async fn result<P: ObservationProvider>(
    State(service): State<StateService<P>>,
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[derive(utoipa::OpenApi)]
#[openapi(paths(create, list, detail, renew, cancel, tasks, result))]
pub struct ObservationApi;
