use super::{auth::node_bearer, error::HttpError};
use crate::execution::Execution;
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::RequestId;
use meshway_protocol::{ErrorResponse, SignedNodeChallengeRequest, execution::*, node::*};

/// Resolve an exact Mesh DNS name under the Node's current permissions.
///
/// Requires a Node session without an instance; unknown and hidden names both return a null record.
/// Reading does not grant execution or extend a lease.
#[utoipa::path(post,path="/agent-api/v1/dns/query",operation_id="queryNodeDns",tag="node",security(("nodeSession"=[])),request_body=NodeDnsQueryRequest,responses((status=200,body=NodeDnsQueryResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn query_dns(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<NodeDnsQueryRequest>, JsonRejection>,
) -> Result<Json<NodeDnsQueryResponse>, HttpError> {
    let token = node_bearer(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .query_dns(token, body, id)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}

/// Allocate a one-time Node challenge from a signed control-key application.
///
/// Verifies the pinned Server and current registered identity before admission.
/// An identical application retry returns the original nonce and deadline without making a consumed challenge usable again.
#[utoipa::path(post,path="/agent-api/v1/auth/challenges",operation_id="createNodeChallenge",tag="node",request_body=SignedNodeChallengeRequest,responses((status=200,body=NodeChallenge),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=400,body=ErrorResponse,description="invalid_request"),(status=409,body=ErrorResponse,description="binding_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn challenge(
    State(service): State<Execution>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<SignedNodeChallengeRequest>, JsonRejection>,
) -> Result<Json<NodeChallenge>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .challenge(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Exchange a fresh control-key proof for a short-lived Node bearer session.
///
/// Consumes the challenge atomically; replaying a consumed proof is rejected.
/// A fresh proof for the same still-valid session intent recovers its token and original deadline.
#[utoipa::path(post,path="/agent-api/v1/auth/session",operation_id="createNodeSession",tag="node",request_body=NodeSessionRequest,responses((status=200,body=NodeSessionResponse),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=400,body=ErrorResponse,description="invalid_request"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
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
/// Rotate the Node's control and data-plane key bindings atomically.
///
/// Requires old and new control-key signatures over the same rotation challenge.
/// An identical completed request can recover still-valid replacement metadata without another rotation or restoring old sessions.
#[utoipa::path(post,path="/agent-api/v1/auth/rotate",operation_id="rotateNodeKeys",tag="node",request_body=NodeRotationRequest,responses((status=200,body=NodeKeyMetadata),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=400,body=ErrorResponse,description="invalid_request"),(status=409,body=ErrorResponse,description="binding_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
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
    ($(#[$doc:meta])* $name:ident,$path:literal,$operation:literal,$request:ty,$response:ty $(, $extra:tt)*) => {
        $(#[$doc])*
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="binding_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable") $(, $extra)*))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<Json<$response>,HttpError> {
            let token=node_bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map(Json).map_err(|e|HttpError::domain(id, e))
        }
    }
}
json_route!(
    /// Acquire the Node's runtime instance with its actual capabilities.
    ///
    /// Requires a Node session; the same instance ID is idempotent only while that instance remains valid in the current Server boot.
    acquire,
    "/agent-api/v1/instances",
    "acquireNodeInstance",
    AcquireInstanceRequest,
    InstanceLease
);
json_route!(
    /// Renew the current Node instance and eligible execution target lease.
    ///
    /// Requires a Node session and matching instance generation; expired demands are reconciled in the same transaction.
    /// Individual business and diagnostic permit deadlines remain independently enforced.
    renew,
    "/agent-api/v1/instances/renew",
    "renewNodeInstance",
    InstanceRequest,
    InstanceLease
);
json_route!(
    /// Retrieve the Node's exact execution target as full content, delta or unchanged.
    ///
    /// Requires a Node session and current instance; the read does not extend the execution lease.
    /// Expired-demand reconciliation may update the publication before the response is assembled.
    configuration,
    "/agent-api/v1/configuration/query",
    "queryNodeConfiguration",
    NodeConfigurationRequest,
    NodeConfigurationResponse
);
json_route!(
    /// Refresh the Node's execution lease and synchronize its matching target atomically.
    ///
    /// Requires a Node session and current instance; renewal and full or delta selection share one transaction.
    /// The Node must validate and persist the target before execution, and report installation separately.
    refresh_configuration,
    "/agent-api/v1/configuration/refresh",
    "refreshNodeConfiguration",
    NodeConfigurationRequest,
    NodeConfigurationResponse
);
json_route!(
    /// Create a business demand for the authenticated source Node.
    ///
    /// Requires a Node session and current instance; requestId binds identical creation retries without extending the demand.
    /// The Server compiles all live demands together and returns feedback requiring target synchronization before execution.
    demand,
    "/agent-api/v1/demands",
    "createNodeDemand",
    DemandRequest,
    DemandResponse
);
json_route!(
    /// Renew an existing live demand without changing its business intent.
    ///
    /// Requires the owning Node session and instance; expiresAt is bounded by the instance deadline and 300 seconds from Server time.
    /// Repeating an equal or earlier deadline does not extend the demand.
    renew_demand,
    "/agent-api/v1/demands/renew",
    "renewNodeDemand",
    RenewDemandRequest,
    DemandResponse
);
macro_rules! empty_route {

    ($(#[$doc:meta])* $name:ident,$path:literal,$operation:literal,$request:ty,$conflicts:literal,$unavailable:literal $(, $extra:tt)*) => {
        $(#[$doc])*
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=409,body=ErrorResponse,description=$conflicts),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description=$unavailable) $(, $extra)*))]
        pub(super) async fn $name(State(service):State<Execution>,Extension(id):Extension<RequestId>,headers:HeaderMap,body:Result<Json<$request>,JsonRejection>)->Result<StatusCode,HttpError> {
            let token=node_bearer(&headers,id)?; let Json(body)=body.map_err(|e|HttpError::json(id,e))?;
            service.$name(token,body).await.map_err(|e|HttpError::domain(id, e))?; Ok(StatusCode::NO_CONTENT)
        }
    }
}
json_route!(
    /// Select an authorized routing profile for the current Node.
    ///
    /// Requires a Node session and current instance; expectedVersion guards the selection with CAS and a mismatch returns binding_conflict.
    /// Acceptance cancels prior source demands and publishes the new selection.
    select_profile,
    "/agent-api/v1/routing-selection",
    "selectNodeRoutingProfile",
    RoutingSelectionRequest,
    RoutingSelectionResponse,
    (status=403,body=ErrorResponse,description="permission_denied")
);
empty_route!(
    /// Release the current Node runtime instance.
    ///
    /// Requires a Node session and matching instance generation; releasing the instance ends its execution qualification.
    release,
    "/agent-api/v1/instances/release",
    "releaseNodeInstance",
    InstanceRequest,
    "binding_conflict, state_conflict",
    "busy, service_unavailable",
    (status=404,body=ErrorResponse,description="not_found"),
    (status=422,body=ErrorResponse,description="configuration_invalid")
);
empty_route!(
    /// Record a Node's actual save or installation result for an exact target.
    ///
    /// Requires a Node session; identical instance, target, attempt and sequence retries retain the original result.
    /// Applied requires an earlier saved result in the same attempt and an eligible lease; receipt_* conflicts identify rejected bindings or preconditions.
    receipt,
    "/agent-api/v1/configuration/receipts",
    "submitNodeConfigurationReceipt",
    ReceiptRequest,
    "receipt_instance_mismatch, receipt_target_mismatch, receipt_publication_superseded, receipt_lease_expired, receipt_precondition, receipt_content_conflict",
    "service_unavailable"
);
empty_route!(
    /// Cancel a business demand owned by the current Node instance.
    ///
    /// Requires a Node session and current instance; remaining live demands are recompiled into the network's execution targets.
    cancel_demand,
    "/agent-api/v1/demands/cancel",
    "cancelNodeDemand",
    CancelDemandRequest,
    "binding_conflict, state_conflict",
    "busy, service_unavailable",
    (status=404,body=ErrorResponse,description="not_found"),
    (status=422,body=ErrorResponse,description="configuration_invalid")
);
