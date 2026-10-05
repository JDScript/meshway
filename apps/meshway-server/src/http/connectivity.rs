use super::{TypedPath, auth::bearer, error::HttpError};
use crate::connectivity::Connectivity;
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_model::{RequestId, UdpTraversalId};
use meshway_protocol::{ErrorResponse, connectivity::*};

macro_rules! query_route {
    ($method:ident, $path:literal, $operation:literal, $request:ty, $response:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="connectivity",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $method(State(service): State<Connectivity>, Extension(id): Extension<RequestId>, headers: HeaderMap, body: Result<Json<$request>, JsonRejection>) -> Result<Json<$response>, HttpError> {
            let token = bearer(&headers, id)?;
            let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
            service.$method(token, body).await.map(Json).map_err(|error| HttpError::domain(id, error))
        }
    };
}
query_route!(
    query,
    "/agent-api/v1/connectivity/udp/query",
    "queryUdpTraversal",
    QueryUdpTraversalRequest,
    QueryUdpTraversalResponse
);
query_route!(
    prepare,
    "/agent-api/v1/connectivity/udp/attempts",
    "prepareUdpTraversal",
    PrepareUdpTraversalRequest,
    PrepareUdpTraversalResponse
);

macro_rules! attempt_route {
    ($method:ident, $path:literal, $operation:literal, $response:ty) => {
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="connectivity",security(("nodeSession"=[])),params(("attemptId"=UdpTraversalId,Path)),request_body=UdpTraversalAttemptRequest,responses((status=200,body=$response),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=503,body=ErrorResponse)))]
        pub(super) async fn $method(State(service): State<Connectivity>, Extension(id): Extension<RequestId>, TypedPath(attempt): TypedPath<UdpTraversalId>, headers: HeaderMap, body: Result<Json<UdpTraversalAttemptRequest>, JsonRejection>) -> Result<Json<$response>, HttpError> {
            let token = bearer(&headers, id)?;
            let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
            service.$method(token, attempt, body).await.map(Json).map_err(|error| HttpError::domain(id, error))
        }
    };
}
attempt_route!(
    accept,
    "/agent-api/v1/connectivity/udp/attempts/{attemptId}/accept",
    "acceptUdpTraversal",
    AcceptUdpTraversalResponse
);
attempt_route!(
    cancel,
    "/agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel",
    "cancelUdpTraversal",
    CancelUdpTraversalResponse
);
