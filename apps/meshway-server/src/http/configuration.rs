use super::{auth::bearer, error::HttpError};

/// Read the complete network configuration JSON Schema.
///
/// Requires an authenticated user session; the document contains definitions and constraints without configuration values.
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
/// List supported transport declarations and their schema references.
///
/// Requires an authenticated user session; catalog entries do not attest a node's runtime support.
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

/// Read the current redacted network configuration.
///
/// Requires current Owner or Admin membership in the network and returns the exact source revision for subsequent edits.
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
/// Read a saved redacted configuration revision.
///
/// Requires current Owner or Admin membership in the network; historical values are returned without applying them.
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
/// Read the complete sensitive JSONC source and canonical configuration.
///
/// Requires current Owner or Admin membership in the network; omitting `revision` selects the current source version.
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
/// Plan one source and target over the current version or a draft.
///
/// Reports the deciding rule, the Access grants that cover the target, the
/// planned paths and why nothing would be permitted. A draft is evaluated as a
/// change to the current version and is not saved.
#[utoipa::path(post,path="/api/v1/networks/{networkId}/configuration/evaluate",operation_id="evaluateConfiguration",tag="configuration",security(("userSession"=[])),params(("networkId"=NetworkId,Path)),request_body=EvaluateConfigurationRequest,responses((status=200,body=EvaluationResponse),(status=400,body=ErrorResponse),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=422,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(super) async fn evaluate(
    State(service): State<Configurations>,
    Extension(id): Extension<RequestId>,
    headers: HeaderMap,
    path: Result<Path<NetworkId>, PathRejection>,
    body: Result<Json<EvaluateConfigurationRequest>, JsonRejection>,
) -> Result<Json<EvaluationResponse>, HttpError> {
    let Path(network) = path.map_err(|_| HttpError::invalid_request(id))?;
    let Json(body) = body.map_err(|e| HttpError::json(id, e))?;
    Ok(Json(
        service
            .evaluate(bearer(&headers, id)?, network, body, id)
            .await
            .map_err(|e| HttpError::domain(id, e))?,
    ))
}
/// Validate a complete configuration edit and preview its effective changes.
///
/// Checks current edit permissions and strict `expectedRevision` without reserving a revision or permanent materials.
/// Returns 409 for a stale baseline and 422 for invalid configuration or unmet preparation requirements.
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
/// Apply a complete configuration edit or historical restore atomically.
///
/// Rechecks edit permissions, dependencies and strict `expectedRevision` in the final transaction.
/// Identical `requestId` retries are idempotent; changed input or a stale baseline returns 409, and invalid configuration returns 422.
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
/// Compare two saved configuration revisions without exposing secret values.
///
/// Requires current Owner or Admin membership in the network and reports source, canonical configuration and stable-binding differences.
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
