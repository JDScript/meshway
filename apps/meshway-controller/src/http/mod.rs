//! Actual HTTP routes and their OpenAPI description share the same registration.

mod auth;
mod configuration;
mod error;
mod management;
mod management_history;
mod management_networks;
mod networks;

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef, Request},
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
use crate::{App, networks::Networks};

use error::HttpError;

/// Includes JSON whitespace and escapes, independently of password limits.
pub const MAX_JSON_BODY_BYTES: usize = 16 * 1024;
/// Full configuration documents include JSONC text and embedded protocol material.
pub const MAX_CONFIGURATION_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct HttpState {
    sealer: std::sync::Arc<crate::secrets::Sealer>,
    accounts: Accounts,
    networks: Networks,
    configurations: crate::configuration::Configurations,
    history: crate::history::History,
}

impl FromRef<HttpState> for std::sync::Arc<crate::secrets::Sealer> {
    fn from_ref(state: &HttpState) -> Self {
        state.sealer.clone()
    }
}

impl FromRef<HttpState> for Accounts {
    fn from_ref(state: &HttpState) -> Self {
        state.accounts.clone()
    }
}

impl FromRef<HttpState> for Networks {
    fn from_ref(state: &HttpState) -> Self {
        state.networks.clone()
    }
}
impl FromRef<HttpState> for crate::configuration::Configurations {
    fn from_ref(state: &HttpState) -> Self {
        state.configurations.clone()
    }
}
impl FromRef<HttpState> for crate::history::History {
    fn from_ref(state: &HttpState) -> Self {
        state.history.clone()
    }
}

fn routes() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(auth::options))
        .routes(routes!(auth::register))
        .routes(routes!(auth::login))
        .routes(routes!(auth::session))
        .routes(routes!(auth::logout))
        .routes(routes!(networks::list, networks::create))
        .routes(routes!(networks::get))
        .routes(routes!(networks::rename))
        .routes(routes!(networks::members))
        .routes(routes!(management::account))
        .routes(routes!(management::sessions))
        .routes(routes!(management::revoke_one))
        .routes(routes!(management::revoke_others))
        .routes(routes!(management::revoke_all))
        .routes(routes!(management::password))
        .routes(routes!(management::email))
        .routes(routes!(management::reset_inspect))
        .routes(routes!(management::reset_redeem))
        .routes(routes!(management_networks::role))
        .routes(routes!(
            management_networks::links,
            management_networks::create_link
        ))
        .routes(routes!(management_networks::inspect_link))
        .routes(routes!(management_networks::redeem_link))
        .routes(routes!(management_networks::revoke_link))
        .routes(routes!(management_networks::removal_impact))
        .routes(routes!(management_networks::leave_impact))
        .routes(routes!(management_networks::remove))
        .routes(routes!(management_networks::leave))
        .merge(
            OpenApiRouter::new()
                .routes(routes!(configuration::get, configuration::apply))
                .routes(routes!(configuration::validate))
                .layer(DefaultBodyLimit::max(MAX_CONFIGURATION_BODY_BYTES)),
        )
        .routes(routes!(configuration::revision))
        .routes(routes!(configuration::source))
        .routes(routes!(configuration::compare))
        .routes(routes!(configuration::schema))
        .routes(routes!(configuration::catalog))
        .routes(routes!(management_history::account))
        .routes(routes!(management_history::network))
        .routes(routes!(management_history::activity))
        .routes(routes!(management_history::revisions))
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

pub fn router(app: &App) -> Router {
    let (router, _) = routes().split_for_parts();
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY_BYTES))
        .layer(middleware::from_fn(context))
        .with_state(HttpState {
            sealer: app.sealer(),
            accounts: app.accounts(),
            networks: app.networks(),
            configurations: app.configurations(),
            history: app.history(),
        })
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
