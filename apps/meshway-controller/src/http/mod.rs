//! Actual HTTP routes and their OpenAPI description share the same registration.

mod auth;
mod error;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use meshway_model::RequestId;
use meshway_protocol::{EmptyParams, Problem};
use utoipa::openapi::{
    OpenApi,
    security::{Http, HttpAuthScheme, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::accounts::{Accounts, random_uuid};

use error::HttpError;

/// Includes JSON whitespace and escapes, independently of password limits.
pub const MAX_JSON_BODY_BYTES: usize = 16 * 1024;

fn routes() -> OpenApiRouter<Accounts> {
    OpenApiRouter::new()
        .routes(routes!(auth::options))
        .routes(routes!(auth::register))
        .routes(routes!(auth::login))
        .routes(routes!(auth::session))
        .routes(routes!(auth::logout))
}

pub fn openapi() -> OpenApi {
    let (_, mut spec) = routes().split_for_parts();
    spec.info.title = "Meshway Controller".into();
    spec.info.version = env!("CARGO_PKG_VERSION").into();
    spec.components
        .get_or_insert_with(Default::default)
        .add_security_scheme(
            "userSession",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    spec
}

pub fn router(accounts: Accounts) -> Router {
    let (router, _) = routes().split_for_parts();
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY_BYTES))
        .layer(middleware::from_fn(context))
        .with_state(accounts)
}

async fn context(mut request: Request, next: Next) -> Response {
    // Ignore client-supplied correlation identifiers; never use credentials or
    // nil IDs as a fallback when the operating system RNG fails.
    let request_id = random_uuid().ok().and_then(|id| RequestId::new(id).ok());
    let mut response = if let Some(id) = request_id {
        request.extensions_mut().insert(id);
        next.run(request).await
    } else {
        HttpError::unavailable(None).into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    if let Some(id) = request_id
        && let Ok(value) = HeaderValue::from_str(&id.to_string())
    {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

async fn not_found(axum::Extension(id): axum::Extension<RequestId>) -> HttpError {
    HttpError::new(
        id,
        StatusCode::NOT_FOUND,
        Problem::NotFound {
            params: EmptyParams {},
            message: "Resource not found.".into(),
        },
    )
}

async fn method_not_allowed(axum::Extension(id): axum::Extension<RequestId>) -> HttpError {
    HttpError::new(
        id,
        StatusCode::METHOD_NOT_ALLOWED,
        Problem::MethodNotAllowed {
            params: EmptyParams {},
            message: "Method not allowed.".into(),
        },
    )
}
