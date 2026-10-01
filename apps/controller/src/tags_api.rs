use axum::{
    Json,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::HeaderMap,
};
use meshway_protocol::{ErrorResponse, SaveTagsRequest, SetNodeTagsRequest, TagSnapshot};
use utoipa_axum::router::OpenApiRouter;

use crate::{AppState, Error, bearer, json};

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_tags, save_tags))
        .routes(utoipa_axum::routes!(set_node_tags))
        .layer(DefaultBodyLimit::max(256 * 1024))
}

#[utoipa::path(get, path = "/api/v1/tags", operation_id = "getTags", security(("adminBearer" = [])),
    responses((status = 200, description = "Admin-owned tag catalog and active assignments", body = TagSnapshot),
    (status = 401, description = "Admin credential required", body = ErrorResponse)))]
async fn get_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TagSnapshot>, Error> {
    let token = bearer(&headers)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.tags().map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/tags", operation_id = "saveTags", security(("adminBearer" = [])), request_body = SaveTagsRequest,
    responses((status = 200, description = "Global-CAS tag catalog save", body = TagSnapshot),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Invalid or referenced tag", body = ErrorResponse)))]
async fn save_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<SaveTagsRequest>, JsonRejection>,
) -> Result<Json<TagSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.save_tags(&request).map(Json)
        })
        .await
}

#[utoipa::path(post, path = "/api/v1/nodes/tags", operation_id = "setNodeTags", security(("adminBearer" = [])), request_body = SetNodeTagsRequest,
    responses((status = 200, description = "Global-CAS active-node tag assignment", body = TagSnapshot),
    (status = 400, description = "Invalid JSON", body = ErrorResponse),
    (status = 401, description = "Admin credential required", body = ErrorResponse),
    (status = 409, description = "Stale global revision", body = ErrorResponse),
    (status = 422, description = "Unknown node or tag", body = ErrorResponse)))]
async fn set_node_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<SetNodeTagsRequest>, JsonRejection>,
) -> Result<Json<TagSnapshot>, Error> {
    let token = bearer(&headers)?;
    let request = json(body)?;
    state
        .access(move |store| {
            store.authorize_admin(&token)?;
            store.set_node_tags(&request).map(Json)
        })
        .await
}
