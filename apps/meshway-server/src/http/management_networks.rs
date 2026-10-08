use super::{auth::bearer, error::HttpError};
use crate::networks::Networks;
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use meshway_model::{JoinLinkId, MembershipId, MembershipRole, NetworkId, RequestId};
use meshway_protocol::{ErrorResponse, NetworkPageQuery, WireSecret, management::*};

/// Preview the User-identity nodes affected by removing a member.
///
/// Requires access to the target's removal impact and returns the membership and configuration concurrency guards.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/members/{membershipId}/removal-impact",operation_id="getMemberRemovalImpact",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("membershipId"=MembershipId,Path)),responses((status=200,body=RemovalImpactResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn removal_impact(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, MembershipId)>, PathRejection>,
) -> Result<Json<RemovalImpactResponse>, HttpError> {
    let Path((network, member)) = path.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        networks
            .removal_impact(bearer(&headers, id)?, network, Some(member), id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Preview the caller's User-identity nodes affected by leaving a network.
///
/// Returns a read-only impact set and the current membership and configuration concurrency guards.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/leave-impact",operation_id="getLeaveImpact",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=RemovalImpactResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn leave_impact(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
) -> Result<Json<RemovalImpactResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        networks
            .removal_impact(bearer(&headers, id)?, network, None, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Remove a network member and permanently delete their current User-identity nodes.
///
/// Rechecks target-role permission, last-Owner protection, and both expected versions in the commit transaction.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/members/{membershipId}/remove",operation_id="removeMember",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("membershipId"=MembershipId,Path)),request_body=RemoveMemberRequest,responses((status=200,body=MemberRemovedResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn remove(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, MembershipId)>, PathRejection>,
    body: Result<Json<RemoveMemberRequest>, JsonRejection>,
) -> Result<Json<MemberRemovedResponse>, HttpError> {
    let Path((network, member)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let revision = networks
        .remove_member(
            bearer(&headers, id)?,
            network,
            Some(member),
            body.expected_version,
            body.expected_revision,
            id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(MemberRemovedResponse {
        request_id: id,
        revision,
    }))
}
/// Leave a network and permanently delete the caller's current User-identity nodes.
///
/// Requires matching membership and configuration versions; the final Owner cannot leave.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/leave",operation_id="leaveNetwork",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=RemoveMemberRequest,responses((status=200,body=MemberRemovedResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn leave(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<RemoveMemberRequest>, JsonRejection>,
) -> Result<Json<MemberRemovedResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let revision = networks
        .remove_member(
            bearer(&headers, id)?,
            network,
            None,
            body.expected_version,
            body.expected_revision,
            id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(MemberRemovedResponse {
        request_id: id,
        revision,
    }))
}

/// Preview join links that a proposed member role change would revoke.
///
/// Uses the same role authorization as the mutation and returns at most 100 affected links plus the total.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/members/{membershipId}/role-change-impact/{role}",operation_id="getMemberRoleChangeImpact",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("membershipId"=MembershipId,Path),("role"=MembershipRole,Path)),responses((status=200,body=RoleChangeImpactResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn role_change_impact(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, MembershipId, MembershipRole)>, PathRejection>,
) -> Result<Json<RoleChangeImpactResponse>, HttpError> {
    let Path((network, member, role)) = path.map_err(|_| HttpError::invalid_request(id))?;
    Ok(Json(
        networks
            .role_change_impact(bearer(&headers, id)?, network, member, role, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Change a member's role using the previously read membership version.
///
/// Enforces assignable roles and last-Owner protection, revoking live links the new role may no longer issue.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/members/{membershipId}/role",operation_id="changeMemberRole",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("membershipId"=MembershipId,Path)),request_body=ChangeMemberRoleRequest,responses((status=200,body=MemberMutationResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn role(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, MembershipId)>, PathRejection>,
    body: Result<Json<ChangeMemberRoleRequest>, JsonRejection>,
) -> Result<Json<MemberMutationResponse>, HttpError> {
    let Path((network, member)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let m = networks
        .change_role(
            bearer(&headers, id)?,
            network,
            member,
            body.expected_version,
            body.role,
            id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(MemberMutationResponse {
        request_id: id,
        member: m,
    }))
}

/// Issue a one-time join link for a role the caller may grant.
///
/// requestId deduplicates matching input; only the first response delivers the secret, while retries return unavailable delivery.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/join-links",operation_id="createJoinLink",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=CreateJoinLinkRequest,responses((status=201,body=JoinLinkCreated),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn create_link(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<CreateJoinLinkRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<JoinLinkCreated>), HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let created = networks
        .create_join_link(
            bearer(&headers, id)?,
            network,
            body.role,
            body.expires_in_seconds,
            body.request_id,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    let result = match created.secret {
        Some(secret) => JoinLinkCreated::Available {
            request_id: id,
            link: created.link,
            secret: WireSecret::new(secret.expose().to_owned()),
        },
        None => JoinLinkCreated::Unavailable {
            request_id: id,
            link: created.link,
        },
    };
    Ok((StatusCode::CREATED, Json(result)))
}

/// List non-secret join link metadata for a network.
///
/// Requires Owner or Admin permission and returns newest-first pages with a default limit of 50.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/join-links",operation_id="listJoinLinks",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),NetworkPageQuery),responses((status=200,body=JoinLinkListResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn links(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    query: Result<Query<NetworkPageQuery>, QueryRejection>,
) -> Result<Json<JoinLinkListResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    let page = networks
        .join_links(
            bearer(&headers, id)?,
            network,
            query.limit.unwrap_or(50),
            query.cursor,
        )
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(JoinLinkListResponse {
        total: page.total,
        request_id: id,
        items: page.items,
        next_cursor: page.next_cursor,
        as_of: page.as_of,
    }))
}

/// Inspect a join link without consuming it.
///
/// Requires a user session and the secret in the request body; inspection does not replace redemption-time checks.
#[utoipa::path(post,path="/api/v1/join-links/inspect",operation_id="inspectJoinLink",tag="members",security(("userSession"=[])),request_body=JoinLinkCredentialRequest,responses((status=200,body=JoinLinkInspection),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn inspect_link(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<JoinLinkCredentialRequest>, JsonRejection>,
) -> Result<Json<JoinLinkInspection>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let result = networks
        .inspect_join_link(bearer(&headers, id)?, body.token.expose())
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(JoinLinkInspection {
        request_id: id,
        link: result.link,
        network_name: result.network_name,
        already_member: result.already_member,
    }))
}

/// Redeem a join link to create a membership for the authenticated user.
///
/// Consumption and membership creation are atomic; same-consumer retries recover only the original still-active membership.
#[utoipa::path(post,path="/api/v1/join-links/redeem",operation_id="redeemJoinLink",tag="members",security(("userSession"=[])),request_body=JoinLinkCredentialRequest,responses((status=200,body=JoinLinkRedeemed),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn redeem_link(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<JoinLinkCredentialRequest>, JsonRejection>,
) -> Result<Json<JoinLinkRedeemed>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    let (network_id, membership_id) = networks
        .redeem_join_link(bearer(&headers, id)?, body.token.expose(), id)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(JoinLinkRedeemed {
        request_id: id,
        network_id,
        membership_id,
    }))
}

/// Revoke a network join link under the caller's current permissions.
///
/// Revocation does not remove a membership created by an already-consumed link.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/join-links/{linkId}/revoke",operation_id="revokeJoinLink",tag="members",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("linkId"=JoinLinkId,Path)),responses((status=204),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn revoke_link(
    State(networks): State<Networks>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, JoinLinkId)>, PathRejection>,
) -> Result<StatusCode, HttpError> {
    let Path((network, link)) = path.map_err(|_| HttpError::invalid_request(id))?;
    networks
        .revoke_join_link(bearer(&headers, id)?, network, link, id)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(StatusCode::NO_CONTENT)
}
