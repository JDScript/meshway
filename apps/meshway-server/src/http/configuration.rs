use super::{auth::bearer, error::HttpError};

#[utoipa::path(get,path="/api/v1/configuration/schema",operation_id="getConfigurationSchema",tag="configuration",security(("userSession"=[])),responses((status=200,body=ConfigurationSchemaResponse),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn schema(
    State(accounts): State<crate::accounts::Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<ConfigurationSchemaResponse>, HttpError> {
    accounts
        .authenticate(bearer(&headers, id)?)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(
        crate::configuration::catalog::schema(id).map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(get,path="/api/v1/transport-catalog",operation_id="getTransportCatalog",tag="configuration",security(("userSession"=[])),responses((status=200,body=TransportCatalogResponse),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn catalog(
    State(accounts): State<crate::accounts::Accounts>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<TransportCatalogResponse>, HttpError> {
    accounts
        .authenticate(bearer(&headers, id)?)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(crate::configuration::catalog::transports(id)))
}
use crate::configuration::Configurations;
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::HeaderMap,
};
use meshway_model::{NetworkId, RequestId, Revision};
use meshway_protocol::{ErrorResponse, configuration::*};

#[utoipa::path(get,path="/api/v1/networks/{networkId}/configuration",operation_id="getConfiguration",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),responses((status=200,body=ConfigurationResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn get(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
) -> Result<Json<ConfigurationResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let snapshot = service
        .read(bearer(&headers, id)?, network, None)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(ConfigurationResponse {
        request_id: id,
        network_id: network,
        revision: snapshot.revision,
        configuration: (&snapshot.document.configuration).into(),
    }))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/configuration/revisions/{revision}",operation_id="getConfigurationRevision",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path),("revision"=Revision,Path)),responses((status=200,body=ConfigurationResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn revision(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<(NetworkId, Revision)>, PathRejection>,
) -> Result<Json<ConfigurationResponse>, HttpError> {
    let Path((network, revision)) = path.map_err(|_| HttpError::invalid_request(id))?;
    let snapshot = service
        .read(bearer(&headers, id)?, network, Some(revision))
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(ConfigurationResponse {
        request_id: id,
        network_id: network,
        revision: snapshot.revision,
        configuration: (&snapshot.document.configuration).into(),
    }))
}
#[utoipa::path(get,path="/api/v1/networks/{networkId}/configuration/source",operation_id="getConfigurationSource",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path),ConfigurationRevisionQuery),responses((status=200,body=ConfigurationSourceResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn source(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    query: Result<Query<ConfigurationRevisionQuery>, QueryRejection>,
) -> Result<Json<ConfigurationSourceResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Query(query) = query.map_err(|_| HttpError::invalid_request(id))?;
    let snapshot = service
        .read(bearer(&headers, id)?, network, query.revision)
        .await
        .map_err(|e| HttpError::domain(id, e))?;
    Ok(Json(crate::configuration::source_response(&snapshot, id)))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/configuration/validate",operation_id="validateConfiguration",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=ConfigurationRequest,responses((status=200,body=ConfigurationPreviewResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=422,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn validate(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<ConfigurationRequest>, JsonRejection>,
) -> Result<Json<ConfigurationPreviewResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .validate(bearer(&headers, id)?, network, body, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/configuration",operation_id="applyConfiguration",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=ConfigurationRequest,responses((status=200,body=ConfigurationAppliedResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=422,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn apply(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<ConfigurationRequest>, JsonRejection>,
) -> Result<Json<ConfigurationAppliedResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .apply(bearer(&headers, id)?, network, body, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
#[utoipa::path(post,path="/api/v1/networks/{networkId}/configuration/compare",operation_id="compareConfiguration",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=CompareConfigurationRequest,responses((status=200,body=ConfigurationComparison),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn compare(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<CompareConfigurationRequest>, JsonRejection>,
) -> Result<Json<ConfigurationComparison>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .compare(bearer(&headers, id)?, network, body, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
