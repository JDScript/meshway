//! HTTP routing, decoding, error responses and live OpenAPI assembly.
mod accounts;
mod configuration;
mod connection_runtime;
mod connections;
mod device_login;
mod endpoint_discovery;
mod latency_history;
mod managed_transports;
mod network_policy;
mod networks;
mod tags;

use crate::{AppState, Error};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as ApiPath, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use meshway_protocol::{
    AgentNodeResponse, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse,
    EnrollAgentRequest, ErrorResponse, HeartbeatRequest, NodeList, Overview, RevokeNodeRequest,
    RevokeNodeResponse, local,
};
use utoipa::openapi::{
    Info, OpenApi,
    security::{Http, HttpAuthScheme, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Conflict => StatusCode::CONFLICT,
            Self::Unprocessable(_) | Self::Capacity(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Body(status, _) => *status,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = if status.is_server_error() {
            "controller storage unavailable".to_owned()
        } else {
            self.to_string()
        };
        let mut response = (status, Json(ErrorResponse { message })).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert("www-authenticate", "Bearer".parse().expect("static header"));
        }
        response
    }
}

fn contract_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_overview))
        .routes(routes!(list_nodes))
        .routes(routes!(create_enrollment_token))
        .routes(routes!(revoke_node))
        .routes(routes!(enroll_agent))
        .routes(routes!(heartbeat_agent))
        .layer(DefaultBodyLimit::max(8192))
        .merge(configuration::routes())
        .merge(connections::routes())
        .merge(connection_runtime::routes())
        .merge(latency_history::routes())
        .merge(endpoint_discovery::routes())
        .merge(tags::routes())
        .merge(networks::routes())
        .merge(device_login::routes())
        .merge(managed_transports::routes())
        .merge(network_policy::routes())
        .merge(accounts::routes())
}

fn describe(api: &mut OpenApi) {
    api.info = Info::new("Meshway control API", env!("CARGO_PKG_VERSION"));
    api.info.description = Some("Durable control-plane identity, public-key associations and configuration intent. Receipts mean saved only; not runtime peer proof, connectivity or tunnel health.".to_owned());
    let components = api.components.get_or_insert_with(Default::default);
    for name in [
        "adminBearer",
        "enrollmentBearer",
        "nodeBearer",
        "userSession",
        "managementBearer",
    ] {
        components.add_security_scheme(
            name,
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
    annotate_references(api);
}

fn annotate_references(api: &mut OpenApi) {
    let mut document = serde_json::to_value(&*api).expect("OpenAPI must serialize");
    let references = [
        (
            "/components/schemas/SetNodeCountryRequest/properties/node_id",
            "node",
        ),
        (
            "/components/schemas/SetNodeTagsRequest/properties/node_id",
            "node",
        ),
        (
            "/components/schemas/NodeTagAssignment/properties/node_id",
            "node",
        ),
        ("/components/schemas/TagDefinition/properties/tag", "tag"),
        (
            "/components/schemas/NodeTagAssignment/properties/tags/items",
            "tag",
        ),
        (
            "/components/schemas/SetNodeTagsRequest/properties/tags/items",
            "tag",
        ),
    ];
    for (pointer, kind) in references {
        let schema = document
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap_or_else(|| panic!("missing schema reference field {pointer}"));
        schema.insert("x-meshway-ref".into(), kind.into());
    }
    *api = serde_json::from_value(document).expect("annotated OpenAPI must deserialize");
}

/// Live routes and the exported OpenAPI share one set of declarations.
pub fn router_and_openapi(state: AppState) -> (Router, OpenApi) {
    let (router, mut api) = contract_router().with_state(state).split_for_parts();
    describe(&mut api);
    (
        router.layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert("cache-control", "no-store".parse().expect("static header"));
                response
            },
        )),
        api,
    )
}

pub fn router(state: AppState) -> Router {
    router_and_openapi(state).0
}

pub fn openapi_json() -> Result<String, serde_json::Error> {
    let (_, mut api) = contract_router().split_for_parts();
    describe(&mut api);
    Ok(format!("{}\n", serde_json::to_string_pretty(&api)?))
}

fn bearer(headers: &HeaderMap) -> Result<String, Error> {
    let mut values = headers.get_all("authorization").iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    let (scheme, token) = value.split_once(' ').ok_or(Error::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || !(local::valid_credential(token)
            || token
                .strip_prefix("mws_")
                .is_some_and(local::valid_credential))
    {
        return Err(Error::Unauthorized);
    }
    Ok(token.to_owned())
}

fn json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Error> {
    body.map(|Json(value)| value)
        .map_err(|rejection| match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Error::Body(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request exceeds endpoint byte budget",
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => Error::Body(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json",
            ),
            _ => Error::Invalid("invalid request JSON"),
        })
}

#[utoipa::path(
    get, path = "/api/v1/networks/{network_id}/overview", operation_id = "getOverview", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Controller identity and total stored nodes including revoked nodes", body = Overview),
        (status = 404, description = "Network not found", body = ErrorResponse),
        (status = 401, description = "Admin credential required", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn get_overview(
    State(state): State<AppState>,
    ApiPath(network_id): ApiPath<String>,
    headers: HeaderMap,
) -> Result<Json<Overview>, Error> {
    let token = bearer(&headers)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Read,
            move |store, network_id| {
                Ok(Json(Overview {
                    name: store.network(network_id)?.name,
                    version: env!("CARGO_PKG_VERSION").into(),
                    node_count: store.count(network_id)?,
                    configuration_revision: store.revision(network_id)?,
                }))
            },
        )
        .await
}

#[utoipa::path(
    get, path = "/api/v1/networks/{network_id}/nodes", operation_id = "listNodes", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    responses((status = 200, description = "Stored nodes sorted by ID", body = NodeList),
        (status = 404, description = "Network not found", body = ErrorResponse),
        (status = 401, description = "Admin credential required", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn list_nodes(
    State(state): State<AppState>,
    ApiPath(network_id): ApiPath<String>,
    headers: HeaderMap,
) -> Result<Json<NodeList>, Error> {
    let token = bearer(&headers)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Read,
            move |store, network_id| {
                Ok(Json(NodeList {
                    nodes: store.nodes(network_id)?,
                }))
            },
        )
        .await
}

#[utoipa::path(
    post, path = "/api/v1/networks/{network_id}/enrollment-tokens", operation_id = "createEnrollmentToken", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    request_body = CreateEnrollmentTokenRequest,
    responses((status = 200, description = "One-use token, plaintext revealed only here", body = CreateEnrollmentTokenResponse),
        (status = 404, description = "Network not found", body = ErrorResponse),
        (status = 400, description = "Invalid request or TTL", body = ErrorResponse),
        (status = 401, description = "Admin credential required", body = ErrorResponse),
        (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
        (status = 415, description = "Expected application/json", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn create_enrollment_token(
    State(state): State<AppState>,
    ApiPath(network_id): ApiPath<String>,
    headers: HeaderMap,
    body: Result<Json<CreateEnrollmentTokenRequest>, JsonRejection>,
) -> Result<Json<CreateEnrollmentTokenResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network_id| {
                store
                    .create_token(network_id, request.expires_in_seconds)
                    .map(Json)
            },
        )
        .await
}

#[utoipa::path(
    post, path = "/api/v1/networks/{network_id}/nodes/revoke", operation_id = "revokeNode", security(("managementBearer" = [])),
    params(("network_id" = String, Path, description = "Stable network ID")),
    request_body = RevokeNodeRequest,
    responses((status = 200, description = "Revoked node; repeated revocation preserves its timestamp", body = RevokeNodeResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Admin credential required", body = ErrorResponse),
        (status = 404, description = "Unknown node", body = ErrorResponse),
        (status = 422, description = "Configuration revision capacity exhausted", body = ErrorResponse),
        (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
        (status = 415, description = "Expected application/json", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn revoke_node(
    State(state): State<AppState>,
    ApiPath(network_id): ApiPath<String>,
    headers: HeaderMap,
    body: Result<Json<RevokeNodeRequest>, JsonRejection>,
) -> Result<Json<RevokeNodeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    if request.node_id.is_empty() || request.node_id.chars().count() > 128 {
        return Err(Error::Invalid("invalid node_id"));
    }
    state
        .network_access(
            token,
            network_id,
            crate::accounts::NetworkPermission::Manage,
            move |store, network_id| {
                Ok(Json(RevokeNodeResponse {
                    node: store.revoke(network_id, &request.node_id)?,
                }))
            },
        )
        .await
}

#[utoipa::path(
    post, path = "/api/v1/agents/enroll", operation_id = "enrollAgent", security(("enrollmentBearer" = [])),
    request_body = EnrollAgentRequest,
    responses((status = 200, description = "Enrolled node, or same active identity on a bound retry", body = AgentNodeResponse),
        (status = 400, description = "Invalid enrollment; token remains unused", body = ErrorResponse),
        (status = 401, description = "Invalid, expired or differently consumed enrollment token", body = ErrorResponse),
        (status = 403, description = "Bound node revoked", body = ErrorResponse),
        (status = 409, description = "Proposed credential already bound", body = ErrorResponse),
        (status = 422, description = "Address, inventory, or configuration capacity exhausted; token remains unused", body = ErrorResponse),
        (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
        (status = 415, description = "Expected application/json", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn enroll_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<EnrollAgentRequest>, JsonRejection>,
) -> Result<Json<AgentNodeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            Ok(Json(AgentNodeResponse {
                node: store.enroll(&token, &request)?,
            }))
        })
        .await
}

#[utoipa::path(
    post, path = "/api/v1/agents/heartbeat", operation_id = "heartbeatAgent", security(("nodeBearer" = [])),
    request_body = HeartbeatRequest,
    responses((status = 200, description = "Credential-scoped process contact; not tunnel health", body = AgentNodeResponse),
        (status = 400, description = "Invalid heartbeat", body = ErrorResponse),
        (status = 401, description = "Node credential required", body = ErrorResponse),
        (status = 403, description = "Node revoked", body = ErrorResponse),
        (status = 413, description = "Request exceeds 8192 bytes", body = ErrorResponse),
        (status = 415, description = "Expected application/json", body = ErrorResponse),
        (status = 500, description = "Storage unavailable", body = ErrorResponse))
)]
async fn heartbeat_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<HeartbeatRequest>, JsonRejection>,
) -> Result<Json<AgentNodeResponse>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    request.validate().map_err(Error::Invalid)?;
    state
        .access(move |store| {
            Ok(Json(AgentNodeResponse {
                node: store.heartbeat(&token, &request.agent_version)?,
            }))
        })
        .await
}
