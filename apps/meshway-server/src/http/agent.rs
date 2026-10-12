use super::{auth::node_bearer, error::HttpError};
use crate::execution::Execution;
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::RequestId;
use meshway_protocol::{
    DnsAssociationAccepted, DnsAssociationReport, ErrorResponse, SignedNodeChallengeRequest,
    execution::*,
    netmap::{SyncRequest, SyncResponse},
    node::*,
};

/// Long-poll the Node's netmap, lease and peer facts.
///
/// Requires a Node session; no instance is needed. The request names the netmap and peer-fact cursor the Node holds.
/// The Server answers at once when either is behind, otherwise holds the request until the netmap or visible facts change, a newer sync of the same Node replaces it, or the hold (about 50 seconds, at most 60) ends.
/// A hold ends at least 15 seconds before the lease the Node holds, and a Node's first sync to a Server process is answered at once.
/// A held base answers `delta`, any other base `full`, the current netmap `unchanged`; every answer carries a fresh lease.
#[utoipa::path(post,path="/agent-api/v1/sync",operation_id="syncNode",tag="execution",security(("nodeSession"=[])),request_body=SyncRequest,responses((status=200,body=SyncResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected, netmap_withdrawn"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn sync(
    State(service): State<crate::netmap::Netmaps>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<SyncRequest>, JsonRejection>,
) -> Result<Json<SyncResponse>, HttpError> {
    let token = node_bearer(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .sync(token, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}

/// Complete a trusted DNS association task as the exit resolver of a source.
///
/// Requires a Node session of an exit whose current netmap (`netmap` must name it) serves `source` with the DNS service the source's current netmap resolves through, under the same selection `generation`.
/// The name must be one a name rule of the source decides; addresses must be public, of `recordType`, sorted and unique; `resolvedAt` at most 30 seconds old; `reportId` is single-use.
/// The association lasts the TTL (at least 30 seconds, at most one hour) from resolution and reaches the source and the exit in their next sync answers.
#[utoipa::path(post,path="/agent-api/v1/dns-associations",operation_id="reportDnsAssociation",tag="execution",security(("nodeSession"=[])),request_body=DnsAssociationReport,responses((status=200,body=DnsAssociationAccepted),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected, permission_denied"),(status=409,body=ErrorResponse,description="request_id_conflict, state_scope_changed"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=429,body=ErrorResponse,description="state_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn dns_association(
    State(service): State<crate::netmap::Netmaps>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<DnsAssociationReport>, JsonRejection>,
) -> Result<Json<DnsAssociationAccepted>, HttpError> {
    let token = node_bearer(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .report_association(token, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}

/// Schema of raw `MWRS` artifact bytes.
pub(super) struct ArtifactBytes;
impl utoipa::PartialSchema for ArtifactBytes {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type};
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
            .into()
    }
}
impl utoipa::ToSchema for ArtifactBytes {}

/// Download one rule-set artifact by digest.
///
/// Requires a Node session. `digest` (`sha256:<64 lowercase hex>`) must be referenced by a netmap the Server stored for the calling Node that can still be under a lease (the current one, or one replaced less than a lease validity ago); otherwise, including an unknown digest, the answer is 404.
/// The body is the immutable artifact; the Node checks its length and digest against the netmap reference before use.
#[utoipa::path(get,path="/agent-api/v1/artifacts/{digest}",operation_id="getRuleSetArtifact",tag="execution",security(("nodeSession"=[])),params(("digest"=String,Path,description="`sha256:` and the lowercase hex SHA-256 of the artifact.")),responses((status=200,content_type="application/vnd.meshway.rules.v1",body=inline(ArtifactBytes),description="The artifact bytes; `ETag` is the digest and `Cache-Control` is `private, max-age=31536000, immutable`."),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn artifact(
    State(service): State<crate::netmap::Netmaps>,
    Extension(id): Extension<RequestId>,
    super::TypedPath(digest): super::TypedPath<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, HttpError> {
    use axum::{http::header, response::IntoResponse};
    let token = node_bearer(&headers, id)?;
    let bytes = service
        .artifact(token, digest.clone())
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok((
        [
            (header::CONTENT_TYPE, meshway_rules::MEDIA_TYPE.to_owned()),
            (header::ETAG, format!("\"{digest}\"")),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Permanently delete the calling Node from its network.
///
/// Requires a Node session; `nodeId` must name the authenticated Node itself.
/// This is the same deletion as a manager's `deleteNode`: the Node's credentials are revoked for good, its netmap is withdrawn, it leaves the inventory and every affected Node is recompiled.
/// Joining again needs a new enrollment.
#[utoipa::path(post,path="/agent-api/v1/leave",operation_id="leaveNode",tag="node",security(("nodeSession"=[])),request_body=NodeLeaveRequest,responses((status=200,body=NodeLeaveResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected, permission_denied"),(status=409,body=ErrorResponse,description="revision_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn leave(
    State(service): State<Execution>,
    State(configurations): State<crate::configuration::Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<NodeLeaveRequest>, JsonRejection>,
) -> Result<Json<NodeLeaveResponse>, HttpError> {
    let token = node_bearer(&headers, id)?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .leave(configurations.committer(), token, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}

/// Allocate a one-time Node challenge from a signed control-key application.
///
/// Verifies the pinned Server and current registered identity before admission.
/// An identical application retry returns the original nonce and deadline without making a consumed challenge usable again.
#[utoipa::path(post,path="/agent-api/v1/auth/challenges",operation_id="createNodeChallenge",tag="node",request_body=SignedNodeChallengeRequest,responses((status=200,body=NodeChallenge),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected"),(status=400,body=ErrorResponse,description="invalid_request"),(status=409,body=ErrorResponse,description="binding_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
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
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=200,body=$response),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="binding_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable") $(, $extra)*))]
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
    /// Renew the current Node instance.
    ///
    /// Requires a Node session and matching instance generation. An instance qualifies State, receipts and coordination; it never bounds the Node's netmap.
    renew,
    "/agent-api/v1/instances/renew",
    "renewNodeInstance",
    InstanceRequest,
    InstanceLease
);
macro_rules! empty_route {

    ($(#[$doc:meta])* $name:ident,$path:literal,$operation:literal,$request:ty,$conflicts:literal,$unavailable:literal $(, $extra:tt)*) => {
        $(#[$doc])*
        #[utoipa::path(post,path=$path,operation_id=$operation,tag="execution",security(("nodeSession"=[])),request_body=$request,responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="node_identity_rejected"),(status=409,body=ErrorResponse,description=$conflicts),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description=$unavailable) $(, $extra)*))]
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
    /// Acceptance recompiles the Node's netmap, delivered by its next sync.
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
    /// Record a Node's actual save or installation result for an exact netmap.
    ///
    /// Requires a Node session; identical instance, netmap, attempt and sequence retries retain the original result.
    /// Applied requires an earlier saved result in the same attempt and the Node's current netmap; receipt_* conflicts identify rejected bindings or preconditions.
    receipt,
    "/agent-api/v1/configuration/receipts",
    "submitNodeConfigurationReceipt",
    ReceiptRequest,
    "receipt_instance_mismatch, receipt_target_mismatch, receipt_netmap_superseded, receipt_precondition, receipt_content_conflict",
    "service_unavailable"
);
