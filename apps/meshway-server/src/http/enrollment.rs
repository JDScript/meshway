use super::{auth::bearer, error::HttpError};

/// Redeem a device's approved enrollment result.
///
/// Requires the device's control-key proof, PKCE verifier and approval code; redemption rechecks eligibility.
#[utoipa::path(post,path="/agent-api/v1/enrollment-requests/{enrollmentRequestId}/redeem",operation_id="redeemEnrollmentRequest",tag="enrollment",params(("enrollmentRequestId"=EnrollmentRequestId,Path)),request_body=RedeemEnrollmentRequest,responses((status=200,body=EnrollmentResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials, invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="revision_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn redeem(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<RedeemEnrollmentRequest>, JsonRejection>,
) -> Result<Json<EnrollmentResponse>, HttpError> {
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .redeem(request, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Register a Node using a one-time Tag enrollment key.
///
/// Requires the enrollment secret and device possession proof; key consumption and Node binding are atomic.
#[utoipa::path(post,path="/agent-api/v1/enrollments",operation_id="enrollNodeWithKey",tag="enrollment",request_body=KeyEnrollmentRequest,responses((status=200,body=EnrollmentResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="binding_conflict, revision_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn redeem_key(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<KeyEnrollmentRequest>, JsonRejection>,
) -> Result<Json<EnrollmentResponse>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .redeem_key(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
use crate::enrollment::Enrollment;
use axum::{
    Extension, Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use meshway_model::{EnrollmentKeyId, EnrollmentRequestId, NetworkId, RequestId};
use meshway_protocol::{ErrorResponse, enrollment::*};
/// Create a signed device enrollment request.
///
/// Requires proof of the device's control key and binds the request to its PKCE challenge.
#[utoipa::path(post,path="/agent-api/v1/enrollment-requests",operation_id="createEnrollmentRequest",tag="enrollment",request_body=CreateEnrollmentRequest,responses((status=200,body=EnrollmentRequestView),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="binding_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
pub(super) async fn create(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    body: Result<Json<CreateEnrollmentRequest>, JsonRejection>,
) -> Result<Json<EnrollmentRequestView>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .create_request(body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Inspect a device authorization request.
///
/// Requires a user session; reading does not claim the request and approval details are permission-filtered.
#[utoipa::path(get,path="/api/v1/device-authorizations/{enrollmentRequestId}",operation_id="getDeviceAuthorization",tag="enrollment",security(("userSession"=[])),params(("enrollmentRequestId"=EnrollmentRequestId,Path)),responses((status=200,body=DeviceAuthorizationView),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn inspect(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
) -> Result<Json<DeviceAuthorizationView>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .inspect(token, request)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Approve a device for enrollment into an eligible network.
///
/// Requires a user session and current enrollment eligibility; an existing device binding fixes the network.
#[utoipa::path(post,path="/api/v1/device-authorizations/{enrollmentRequestId}/approve",operation_id="approveDeviceAuthorization",tag="enrollment",security(("userSession"=[])),params(("enrollmentRequestId"=EnrollmentRequestId,Path)),request_body=ApproveEnrollmentRequest,responses((status=200,body=DeviceAuthorizationView),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn approve(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<ApproveEnrollmentRequest>, JsonRejection>,
) -> Result<Json<DeviceAuthorizationView>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .approve(token, request, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Withdraw the current user's unredeemed device approval.
///
/// Requires the approving user's session; incompatible lifecycle states return 409 state_conflict.
#[utoipa::path(post,path="/api/v1/device-authorizations/{enrollmentRequestId}/deny",operation_id="denyDeviceAuthorization",tag="enrollment",security(("userSession"=[])),params(("enrollmentRequestId"=EnrollmentRequestId,Path)),responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn deny(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
) -> Result<StatusCode, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .deny(token, request)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
/// Query an enrollment outcome using device possession proof.
///
/// Requires the request's control-key proof and PKCE verifier; invalid proof returns 401.
#[utoipa::path(post,path="/agent-api/v1/enrollment-requests/{enrollmentRequestId}/query",operation_id="queryEnrollmentRequest",tag="enrollment",params(("enrollmentRequestId"=EnrollmentRequestId,Path)),request_body=EnrollmentDeviceProof,responses((status=200,body=EnrollmentPollResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=404,body=ErrorResponse,description="not_found"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn poll(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<EnrollmentDeviceProof>, JsonRejection>,
) -> Result<Json<EnrollmentPollResponse>, HttpError> {
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .poll(request, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Cancel a device enrollment request.
///
/// Requires the requesting device's possession proof; incompatible lifecycle states return 409 state_conflict.
#[utoipa::path(post,path="/agent-api/v1/enrollment-requests/{enrollmentRequestId}/cancel",operation_id="cancelEnrollmentRequest",tag="enrollment",params(("enrollmentRequestId"=EnrollmentRequestId,Path)),request_body=EnrollmentDeviceProof,responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_credentials"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn cancel(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    path: Result<Path<EnrollmentRequestId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<EnrollmentDeviceProof>, JsonRejection>,
) -> Result<StatusCode, HttpError> {
    let Path(request) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .cancel(request, body)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
/// Create a one-time enrollment key with preassigned Tags.
///
/// Requires a network administrator's user session; initial settings are validated before creation.
/// Idempotent retries return metadata with secretDelivery unavailable.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/enrollment-keys",operation_id="createEnrollmentKey",tag="enrollment",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=CreateEnrollmentKeyRequest,responses((status=200,body=EnrollmentKeyResponse),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="request_id_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=422,body=ErrorResponse,description="configuration_invalid"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn create_key(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, axum::extract::rejection::PathRejection>,
    body: Result<Json<CreateEnrollmentKeyRequest>, JsonRejection>,
) -> Result<Json<EnrollmentKeyResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    service
        .create_key(token, network, body)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// List non-secret enrollment key metadata.
///
/// Requires a network administrator's user session; the requested page size is 1–200, defaulting to 50.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/enrollment-keys",operation_id="listEnrollmentKeys",tag="enrollment",security(("userSession"=[])),params(("networkId"=NetworkId,Path),meshway_protocol::NetworkPageQuery),responses((status=200,body=EnrollmentKeysResponse),(status=400,body=ErrorResponse,description="invalid_cursor, invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn list_keys(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, axum::extract::rejection::PathRejection>,
    query: Result<
        axum::extract::Query<meshway_protocol::NetworkPageQuery>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<EnrollmentKeysResponse>, HttpError> {
    let token = bearer(&headers, id)?;
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let axum::extract::Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    service
        .list_keys(token, network, query.limit.unwrap_or(50), query.cursor)
        .await
        .map(Json)
        .map_err(|e| HttpError::domain(id, e))
}
/// Revoke an enrollment key.
///
/// Requires a network administrator's user session; revocation does not delete an already enrolled Node.
#[utoipa::path(delete,path="/api/v1/networks/{networkId}/enrollment-keys/{keyId}",operation_id="revokeEnrollmentKey",tag="enrollment",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("keyId"=EnrollmentKeyId,Path)),responses((status=204),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="state_conflict"),(status=503,body=ErrorResponse,description="service_unavailable")))]
pub(super) async fn revoke_key(
    State(service): State<Enrollment>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, EnrollmentKeyId)>, axum::extract::rejection::PathRejection>,
) -> Result<StatusCode, HttpError> {
    let token = bearer(&headers, id)?;
    let Path((network, key)) = path.map_err(|_| HttpError::invalid_request(id))?;
    service
        .revoke_key(token, network, key)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
