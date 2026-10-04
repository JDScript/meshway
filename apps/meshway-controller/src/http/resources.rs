use super::TypedPath as Path;
use super::{
    error::HttpError,
    state::{failure, user},
};
use crate::resources::service::{ResourceProvider, ResourceService};
use axum::{Extension, Json, Router, extract::State, http::HeaderMap, middleware};
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{
    ErrorResponse,
    resources::{RuleSetList, RuleSetView},
};
pub fn router<P: ResourceProvider>(service: ResourceService<P>) -> Router {
    use super::declared_route;
    let router = declared_route::<__path_list, _, _, _>(Router::new(), list::<P>);
    let router = declared_route::<__path_detail, _, _, _>(router, detail::<P>);
    declared_route::<__path_refresh, _, _, _>(router, refresh::<P>)
        .layer(middleware::from_fn(super::context))
        .with_state(service)
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/rule-sets",operation_id="listRuleSets",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=RuleSetList),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}",operation_id="getRuleSet",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("ruleSetKey"=String,Path)),responses((status=200,body=RuleSetList),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
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
            .map_err(|e| failure(id, e))?,
    ))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/refresh",operation_id="refreshRuleSet",tag="resources",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("ruleSetKey"=String,Path)),responses((status=200,body=RuleSetView),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn refresh<P: ResourceProvider>(
    State(service): State<ResourceService<P>>,
    Extension(id): Extension<RequestId>,
    Path((network, key)): Path<(NetworkId, String)>,
    headers: HeaderMap,
) -> Result<Json<RuleSetView>, HttpError> {
    Ok(Json(
        service
            .refresh_rule_set(user(&headers, network, id)?, key)
            .await
            .map_err(|e| failure(id, e))?,
    ))
}
#[derive(utoipa::OpenApi)]
#[openapi(paths(list, detail, refresh))]
pub struct ResourceApi;
