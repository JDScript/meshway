use super::{TypedPath, auth::node_bearer, error::HttpError};
use crate::connectivity::Connectivity;
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_model::{RequestId, UdpTraversalId};
use meshway_protocol::{ErrorResponse, connectivity::*};

macro_rules! query_route {
    ($(#[$doc:meta])* $method:ident, $path:literal, $operation:literal, $request:ty, $response:ty) => {
        $(#[$doc])*
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="connectivity",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
        pub(super) async fn $method(State(service): State<Connectivity>, Extension(id): Extension<RequestId>, headers: HeaderMap, body: Result<Json<$request>, JsonRejection>) -> Result<Json<$response>, HttpError> {
            let token = node_bearer(&headers, id)?;
            let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
            service.$method(token, body).await.map(Json).map_err(|error| HttpError::domain(id, error))
        }
    };
}
query_route!(
    /// Query current STUN metadata and the Node's valid UDP acquisition attempts.
    ///
    /// Requires a Node session and exact current instance and execution target.
    /// The query preserves original candidate and attempt deadlines.
    query,
    "/agent-api/v1/connectivity/udp/query",
    "queryUdpTraversal",
    QueryUdpTraversalRequest,
    QueryUdpTraversalResponse
);
query_route!(
    /// Prepare an immutable UDP acquisition offer for an authorized adjacent peer.
    ///
    /// Requires a Node session, current execution scope and exact State-owned candidates from both participants.
    /// Identical requestId retries preserve the offer and deadline; changed input returns unavailable with requestChanged.
    prepare,
    "/agent-api/v1/connectivity/udp/attempts",
    "prepareUdpTraversal",
    PrepareUdpTraversalRequest,
    PrepareUdpTraversalResponse
);

macro_rules! attempt_route {
    ($(#[$doc:meta])* $method:ident, $path:literal, $operation:literal, $response:ty) => {
        $(#[$doc])*
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="connectivity",security(("nodeSession"=[])),params(("attemptId"=UdpTraversalId,Path)),request_body=UdpTraversalAttemptRequest,responses((status=200,body=$response),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
        pub(super) async fn $method(State(service): State<Connectivity>, Extension(id): Extension<RequestId>, TypedPath(attempt): TypedPath<UdpTraversalId>, headers: HeaderMap, body: Result<Json<UdpTraversalAttemptRequest>, JsonRejection>) -> Result<Json<$response>, HttpError> {
            let token = node_bearer(&headers, id)?;
            let Json(body) = body.map_err(|error| HttpError::json(id, error))?;
            service.$method(token, attempt, body).await.map(Json).map_err(|error| HttpError::domain(id, error))
        }
    };
}
attempt_route!(
    /// Accept an exact UDP offer after reserving the actual receiver slot.
    ///
    /// Requires a participant's Node session and matching instance, configuration, State epoch, offer version and digest.
    /// Active is returned only after both participants accept; it permits bounded acquisition under the original deadline.
    accept,
    "/agent-api/v1/connectivity/udp/attempts/{attemptId}/accept",
    "acceptUdpTraversal",
    AcceptUdpTraversalResponse
);
attempt_route!(
    /// Cancel an unfinished UDP acquisition attempt idempotently.
    ///
    /// Requires a participant's Node session and the offer's exact CAS bindings.
    /// Cancellation ends acquisition without withdrawing an independently authenticated business channel.
    cancel,
    "/agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel",
    "cancelUdpTraversal",
    CancelUdpTraversalResponse
);
