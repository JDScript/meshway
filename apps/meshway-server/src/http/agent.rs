use super::{auth::bearer, error::HttpError};
use crate::execution::Execution;
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::RequestId;
use meshway_protocol::{ErrorResponse, execution::*, node::*};

#[utoipa::path(post,path="/agent-api/v1/dns/query",operation_id="queryNodeDns",tag="node",security(("nodeSession"=[])),request_body=NodeDnsQueryRequest,responses((status=200,body=NodeDnsQueryResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn query_dns(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<NodeDnsQueryRequest>, JsonRejection>,
) -> Result<Json<NodeDnsQueryResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .query_dns(token, body, id)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}

#[utoipa::path(post,path="/agent-api/v1/auth/challenges",operation_id="createNodeChallenge",tag="node",request_body=NodeChallengeRequest,responses((status=200,body=NodeChallenge),(status=401,body=ErrorResponse),(status=400,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn challenge(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<NodeChallengeRequest>, JsonRejection>,
) -> Result<Json<NodeChallenge>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .challenge(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
#[utoipa::path(post,path="/agent-api/v1/auth/session",operation_id="createNodeSession",tag="node",request_body=NodeSessionRequest,responses((status=200,body=NodeSessionResponse),(status=401,body=ErrorResponse),(status=400,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn session(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<NodeSessionRequest>, JsonRejection>,
) -> Result<Json<NodeSessionResponse>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .session(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
#[utoipa::path(post,path="/agent-api/v1/auth/rotate",operation_id="rotateNodeKeys",tag="node",request_body=NodeRotationRequest,responses((status=200,body=NodeKeyMetadata),(status=401,body=ErrorResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn rotate(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<NodeRotationRequest>, JsonRejection>,
) -> Result<Json<NodeKeyMetadata>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .rotate(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
macro_rules! json_route {
    ($name:ident,$path:literal,$operation:literal,$request:ty,$response:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<Json<$response>,HttpError> {
            let token=bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map(Json).map_err(|e|HttpError::domain(id, e))
        }
    }
}
json_route!(
    acquire,
    "/agent-api/v1/instances",
    "acquireNodeInstance",
    AcquireInstanceRequest,
    InstanceLease
);
json_route!(
    renew,
    "/agent-api/v1/instances/renew",
    "renewNodeInstance",
    InstanceRequest,
    InstanceLease
);
json_route!(
    configuration,
    "/agent-api/v1/configuration/query",
    "queryNodeConfiguration",
    NodeConfigurationRequest,
    NodeConfigurationResponse
);
json_route!(
    refresh_configuration,
    "/agent-api/v1/configuration/refresh",
    "refreshNodeConfiguration",
    NodeConfigurationRequest,
    NodeConfigurationResponse
);
json_route!(
    demand,
    "/agent-api/v1/demands",
    "createNodeDemand",
    DemandRequest,
    DemandResponse
);
json_route!(
    renew_demand,
    "/agent-api/v1/demands/renew",
    "renewNodeDemand",
    RenewDemandRequest,
    DemandResponse
);
macro_rules! empty_route {

    ($name:ident,$path:literal,$operation:literal,$request:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=204),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<StatusCode,HttpError> {
            let token=bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map_err(|e|HttpError::domain(id, e))?; Ok(StatusCode::NO_CONTENT)
        }
    }
}
json_route!(
    select_profile,
    "/agent-api/v1/routing-selection",
    "selectNodeRoutingProfile",
    RoutingSelectionRequest,
    RoutingSelectionResponse
);
empty_route!(
    release,
    "/agent-api/v1/instances/release",
    "releaseNodeInstance",
    InstanceRequest
);
empty_route!(
    receipt,
    "/agent-api/v1/configuration/receipts",
    "submitNodeConfigurationReceipt",
    ReceiptRequest
);
empty_route!(
    cancel_demand,
    "/agent-api/v1/demands/cancel",
    "cancelNodeDemand",
    CancelDemandRequest
);
