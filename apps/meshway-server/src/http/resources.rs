use super::TypedPath as Path;
use super::{error::HttpError, state::user};
use crate::resources::service::{ResourceProvider, ResourceService};
use axum::{
    Extension, Json, Router,
    extract::{State, rejection::JsonRejection},
    http::HeaderMap,
    middleware,
};
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{
    ErrorResponse,
    resources::{RefreshRuleSetRequest, RuleSetList, RuleSetReport, RuleSetView},
};
pub fn router<P: ResourceProvider>(service: ResourceService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_list, _, _, _>(Router::new(), list::<P>);
    let router = declared_route::<__path_detail, _, _, _>(router, detail::<P>);
    let router = declared_route::<__path_report, _, _, _>(router, report::<P>);
    declared_route::<__path_refresh, _, _, _>(router, refresh::<P>)
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
/// List the network's rule-set definitions and current check results.
///
/// Requires current Owner or Admin membership; artifact creation and successful check times remain distinct.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/rule-sets",operation_id="listRuleSets",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=RuleSetList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
async fn list<P: ResourceProvider>(
    State(service): State<ResourceService<P>>,
    Extension(id): Extension<RequestId>,
    Path(network): Path<NetworkId>,
    headers: HeaderMap,
) -> Result<Json<RuleSetList>, HttpError> {
    Ok(Json(
        service
            .rule_sets(user(&headers, network, id)?, None)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Read one rule set's definition, selected artifact and check status.
///
/// Requires current Owner or Admin membership and returns a one-item list; an unknown key returns 404.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}",operation_id="getRuleSet",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("ruleSetKey"=String,Path)),responses((status=200,body=RuleSetList),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
async fn detail<P: ResourceProvider>(
    State(service): State<ResourceService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, key)): Path<(NetworkId, String)>,
    headers: HeaderMap,
) -> Result<Json<RuleSetList>, HttpError> {
    Ok(Json(
        service
            .rule_sets(user(&headers, network, id)?, Some(key))
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Read the complete conversion report of one rule set's bound content.
///
/// Requires current Owner or Admin membership. The report lists the first 1,000 skipped source entries with position, reason and text; a catalog set whose published report could not be read lists none and keeps its counts. A key without bound content returns 404.
#[utoipa::path(get,path="/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/report",operation_id="getRuleSetReport",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("ruleSetKey"=String,Path)),responses((status=200,body=RuleSetReport),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=503,body=ErrorResponse,description="service_unavailable")))]
async fn report<P: ResourceProvider>(
    State(service): State<ResourceService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, key)): Path<(NetworkId, String)>,
    headers: HeaderMap,
) -> Result<Json<RuleSetReport>, HttpError> {
    Ok(Json(
        service
            .rule_set_report(user(&headers, network, id)?, key)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Refresh a remote or catalog rule set and publish validated content changes.
///
/// Requires current Owner or Admin membership, rechecked before binding the downloaded content.
/// For 24 hours, same-user and same-network `requestId` retries for this key return its current view; reuse for another key returns 409.
/// Failed checks preserve the last validated artifact and report the problem in the returned view.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/refresh",operation_id="refreshRuleSet",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("ruleSetKey"=String,Path)),request_body=RefreshRuleSetRequest,responses((status=200,body=RuleSetView),(status=400,body=ErrorResponse,description="invalid_request"),(status=401,body=ErrorResponse,description="invalid_session"),(status=403,body=ErrorResponse,description="permission_denied"),(status=404,body=ErrorResponse,description="not_found"),(status=409,body=ErrorResponse,description="request_id_conflict, state_conflict"),(status=413,body=ErrorResponse,description="request_too_large"),(status=415,body=ErrorResponse,description="unsupported_media_type"),(status=429,body=ErrorResponse,description="state_limit"),(status=503,body=ErrorResponse,description="busy, service_unavailable")))]
async fn refresh<P: ResourceProvider>(
    State(service): State<ResourceService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, key)): Path<(NetworkId, String)>,
    headers: HeaderMap,
    body: Result<Json<RefreshRuleSetRequest>, JsonRejection>,
) -> Result<Json<RuleSetView>, HttpError> {
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .refresh_rule_set(user(&headers, network, id)?, key, body.request_id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[derive(utoipa::OpenApi)]
#[openapi(paths(list, detail, report, refresh))]
pub struct ResourceApi;
