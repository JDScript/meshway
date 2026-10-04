//! Actual HTTP routes and their OpenAPI description share the same registration.

mod agent;
mod auth;
mod configuration;
mod enrollment;
mod error;
pub mod events;
mod management;
mod management_history;
mod management_networks;
mod networks;
mod nodes;
pub mod observations;
pub mod resources;
pub mod state;

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

struct TypedPath<T>(T);
impl<S, T> axum::extract::FromRequestParts<S> for TypedPath<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned + Send,
{
    type Rejection = HttpError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let id = parts
            .extensions
            .get::<RequestId>()
            .copied()
            .ok_or_else(|| HttpError::unavailable(None))?;
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Self(value))
            .map_err(|_| HttpError::invalid_request(id))
    }
}

/// Generic provider handlers use the same Utoipa path/method declarations as
/// their export; no second literal endpoint registry is maintained.
fn declared_route<D, H, T, S>(router: Router<S>, handler: H) -> Router<S>
where
    D: utoipa::Path,
    H: axum::handler::Handler<T, S>,
    T: 'static,
    S: Clone + Send + Sync + 'static,
{
    use utoipa_axum::PathItemExt;
    let methods = D::methods()
        .iter()
        .fold(axum::routing::MethodRouter::new(), |router, method| {
            router.on(method.to_method_filter(), handler.clone())
        });
    router.route(&D::path(), methods)
}

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
    execution: crate::execution::Execution,
    enrollment: crate::enrollment::Enrollment,
}
impl FromRef<HttpState> for crate::execution::Execution {
    fn from_ref(state: &HttpState) -> Self {
        state.execution.clone()
    }
}
impl FromRef<HttpState> for crate::enrollment::Enrollment {
    fn from_ref(state: &HttpState) -> Self {
        state.enrollment.clone()
    }
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
        .routes(routes!(agent::challenge))
        .routes(routes!(agent::session))
        .routes(routes!(agent::rotate))
        .routes(routes!(agent::acquire))
        .routes(routes!(agent::renew))
        .routes(routes!(agent::release))
        .routes(routes!(agent::configuration))
        .routes(routes!(agent::receipt))
        .routes(routes!(agent::demand))
        .routes(routes!(agent::select_profile))
        .routes(routes!(agent::cancel_demand))
        .routes(routes!(enrollment::create))
        .routes(routes!(enrollment::inspect))
        .routes(routes!(enrollment::approve))
        .routes(routes!(enrollment::deny))
        .routes(routes!(enrollment::poll))
        .routes(routes!(enrollment::cancel))
        .routes(routes!(enrollment::redeem))
        .routes(routes!(enrollment::redeem_key))
        .routes(routes!(enrollment::create_key, enrollment::list_keys))
        .routes(routes!(enrollment::revoke_key))
        .routes(routes!(nodes::list))
        .routes(routes!(nodes::get, nodes::delete))
        .routes(routes!(nodes::revoke_key))
        .routes(routes!(nodes::progress))
}

pub fn openapi() -> OpenApi {
    use utoipa::OpenApi as _;
    let (_, mut spec) = routes().split_for_parts();
    let mut state_spec = state::StateApi::openapi();
    events::close_state_unions(&mut state_spec);
    spec.merge(state_spec);
    spec.merge(state::ProgressApi::openapi());
    events::declare_sse::<meshway_protocol::UserEvent>(
        &mut spec,
        &<state::__path_user_events as utoipa::Path>::path(),
    )
    .expect("registered user stream");
    events::declare_sse::<meshway_protocol::NodeEvent>(
        &mut spec,
        &<state::__path_node_events as utoipa::Path>::path(),
    )
    .expect("registered node stream");
    spec.info.title = "Meshway Controller".into();
    spec.components
        .get_or_insert_with(Default::default)
        .add_security_scheme(
            "nodeSession",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
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
            execution: app.execution(),
            enrollment: app.enrollment(),
        })
        .merge(state::user_router(
            app.user_state_service().expect("App state runtime"),
        ))
        .merge(state::node_router(
            app.node_state_service().expect("App state runtime"),
        ))
        .merge(state::progress_router(
            app.user_state_service().expect("App state runtime"),
        ))
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
