use super::{auth::bearer, error::HttpError};
use crate::execution::{Execution, NodeError};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::RequestId;
use meshway_protocol::{EmptyParams, ErrorResponse, Problem, execution::*, node::*};

pub(super) fn error(id: RequestId, e: NodeError) -> HttpError {
    let (status, problem) = match e {
        NodeError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            Problem::InvalidCredentials {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::Forbidden => (
            StatusCode::FORBIDDEN,
            Problem::PermissionDenied {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::Invalid => return HttpError::invalid_request(id),
        NodeError::InvalidSettings(issues) => {
            return HttpError::configuration(
                id,
                crate::configuration::Error::Invalid(
                    meshway_configuration::ConfigurationError::Invalid(issues),
                ),
            );
        }
        NodeError::Conflict => (
            StatusCode::CONFLICT,
            Problem::RequestConflict {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::VersionConflict => (
            StatusCode::CONFLICT,
            Problem::VersionConflict {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::StateConflict => (
            StatusCode::CONFLICT,
            Problem::StateConflict {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::InvalidCursor => (
            StatusCode::BAD_REQUEST,
            Problem::InvalidCursor {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::NotFound => (
            StatusCode::NOT_FOUND,
            Problem::NotFound {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::Busy => (
            StatusCode::SERVICE_UNAVAILABLE,
            Problem::Busy {
                params: EmptyParams {},
                message: e.to_string(),
            },
        ),
        NodeError::Storage => return HttpError::unavailable(Some(id)),
    };
    HttpError::new(id, status, problem)
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
        .map_err(|e| error(id, e))
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
        .map_err(|e| error(id, e))
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
        .map_err(|e| error(id, e))
}
macro_rules! json_route {
    ($name:ident,$path:literal,$operation:literal,$request:ty,$response:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<Json<$response>,HttpError> {
            let token=bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map(Json).map_err(|e|error(id,e))
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
    demand,
    "/agent-api/v1/demands",
    "createNodeDemand",
    DemandRequest,
    DemandResponse
);
macro_rules! empty_route {

    ($name:ident,$path:literal,$operation:literal,$request:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=204),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<StatusCode,HttpError> {
            let token=bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map_err(|e|error(id,e))?; Ok(StatusCode::NO_CONTENT)
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
