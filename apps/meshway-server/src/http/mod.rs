//! Actual HTTP routes and their OpenAPI description share the same registration.

mod agent;
mod auth;
mod configuration;
mod connectivity;
mod delegation;
mod enrollment;
mod error;
pub mod events;
mod examples;
mod management;
mod management_history;
mod management_networks;
mod networks;
mod nodes;
pub mod observations;
pub mod resources;
mod schema;
pub mod state;

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef, MatchedPath, Request},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use meshway_model::RequestId;
use meshway_protocol::{EmptyParams, Problem};
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::{Level, field};
use utoipa::openapi::{
    OpenApi,
    security::{Http, HttpAuthScheme, SecurityScheme},
};
use utoipa_axum::router::OpenApiRouter;

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

/// The origin delegation clients should use for this Server, if configured.
#[derive(Clone)]
pub(super) struct PublicOrigin(Option<std::sync::Arc<crate::settings::Url>>);

#[derive(Clone)]
struct HttpState {
    sealer: std::sync::Arc<crate::secrets::Sealer>,
    accounts: Accounts,
    public_origin: PublicOrigin,
    networks: Networks,
    configurations: crate::configuration::Configurations,
    history: crate::history::History,
    execution: crate::execution::Execution,
    connectivity: crate::connectivity::Connectivity,
    enrollment: crate::enrollment::Enrollment,
}
impl FromRef<HttpState> for crate::execution::Execution {
    fn from_ref(state: &HttpState) -> Self {
        state.execution.clone()
    }
}
impl FromRef<HttpState> for crate::connectivity::Connectivity {
    fn from_ref(state: &HttpState) -> Self {
        state.connectivity.clone()
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
impl FromRef<HttpState> for PublicOrigin {
    fn from_ref(state: &HttpState) -> Self {
        state.public_origin.clone()
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
    let mut schema_registry = schema::Registry::default();
    // Each expansion builds its operations and schemas in its own frame.
    // Inlined into `routes()`, the temporaries of every route would share one
    // frame, which debug builds do not reuse; that frame outgrew a 2 MiB test
    // thread stack.
    #[inline(never)]
    fn framed<T>(build: impl FnOnce() -> T) -> T {
        build()
    }
    macro_rules! routes {
        ($($handler:tt)*) => {{
            let route = framed(|| utoipa_axum::routes!($($handler)*));
            schema_registry.add(route.0.iter().map(|(name, schema)| (name, schema)));
            route
        }};
    }
    OpenApiRouter::new()
        .routes(routes!(auth::options))
        .routes(routes!(auth::register))
        .routes(routes!(auth::login))
        .routes(routes!(auth::session))
        .routes(routes!(auth::logout))
        .routes(routes!(delegation::create))
        .routes(routes!(delegation::exchange))
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
        .routes(routes!(management_networks::role_change_impact))
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
                .routes(routes!(configuration::evaluate))
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
        .routes(routes!(agent::refresh_configuration))
        .routes(routes!(agent::query_dns))
        .routes(routes!(connectivity::query))
        .routes(routes!(connectivity::prepare))
        .routes(routes!(connectivity::accept))
        .routes(routes!(connectivity::cancel))
        .routes(routes!(agent::receipt))
        .routes(routes!(agent::demand))
        .routes(routes!(agent::renew_demand))
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
        .routes(routes!(nodes::settings))
        .routes(routes!(nodes::get, nodes::delete))
        .routes(routes!(nodes::revoke_key))
        .routes(routes!(nodes::progress))
}

pub use error::{ProblemSample, problem_samples};

pub fn openapi() -> OpenApi {
    use utoipa::OpenApi as _;
    let (_, mut spec) = routes().split_for_parts();
    let mut state_spec = state::StateApi::openapi();
    events::close_state_unions(&mut state_spec);
    schema::merge(&mut spec, state_spec);
    schema::merge(&mut spec, state::ProgressApi::openapi());
    schema::merge(&mut spec, resources::ResourceApi::openapi());
    schema::merge(&mut spec, observations::ObservationApi::openapi());
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
    spec.info.title = "Meshway Server".into();
    spec.info.description = Some(
        "Console API under /api/v1 (user sessions) and Node API under /agent-api/v1 \
         (node sessions), generated from the actual routes."
            .into(),
    );
    spec.info.contact = None;
    spec.info.license = None;
    spec.tags = Some(
        [
            ("auth", "Registration, login and sessions"),
            (
                "account",
                "The signed-in account, its sessions and activity",
            ),
            ("networks", "Networks and membership listing"),
            (
                "members",
                "Membership roles, join links, removal and leaving",
            ),
            (
                "configuration",
                "Network configuration: read, validate, apply, evaluate, history",
            ),
            (
                "nodes",
                "Node inventory, settings, keys and deployment progress",
            ),
            ("enrollment", "Device authorization and enrollment keys"),
            ("state", "Network State projections for users and Nodes"),
            ("events", "SSE streams"),
            ("resources", "Rule sets"),
            ("history", "Activity and observation history"),
            ("observations", "Realtime observations"),
            (
                "execution",
                "Node execution: instances, demands, receipts, publications",
            ),
            ("node", "Node authentication and key rotation"),
            ("connectivity", "UDP traversal coordination"),
        ]
        .into_iter()
        .map(|(name, description)| {
            utoipa::openapi::tag::TagBuilder::new()
                .name(name)
                .description(Some(description))
                .build()
        })
        .collect(),
    );
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
    examples::attach(&mut spec);
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
            public_origin: PublicOrigin(app.public_origin().cloned().map(std::sync::Arc::new)),
            networks: app.networks(),
            configurations: app.configurations(),
            history: app.history(),
            execution: app.execution(),
            connectivity: app.connectivity(),
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
        .merge(resources::router(app.resources()))
        .merge(observations::router(app.observations()))
        .layer(cors(app.accounts().delegation_clients()))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request| {
                    // Only the declared route template is safe here: raw URIs,
                    // queries, headers and bodies may contain private inputs.
                    let route = request.extensions().get::<MatchedPath>()
                        .map_or("<unmatched>", MatchedPath::as_str);
                    tracing::info_span!("http_request", method = %request.method(), route, request_id = field::Empty)
                })
                .on_request(())
                .on_response(DefaultOnResponse::new().level(Level::DEBUG))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
}

/// Browsers on a configured delegation client's origin may call `/api/v1`
/// with a bearer token; nothing else is reachable cross-origin and cookies
/// never take part. Without clients no CORS headers are sent at all.
fn cors(clients: &crate::accounts::delegation::DelegationClients) -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    if clients.is_empty() {
        return CorsLayer::new();
    }
    let origins: Vec<HeaderValue> = clients
        .origins()
        .filter_map(|origin| HeaderValue::from_str(origin.as_str().trim_end_matches('/')).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, parts| {
            parts.uri.path().starts_with("/api/v1/") && origins.iter().any(|o| o == origin)
        }))
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
        ])
        .expose_headers([HeaderName::from_static("x-request-id"), header::DATE])
        .max_age(std::time::Duration::from_secs(600))
}

async fn context(mut request: Request, next: Next) -> Response {
    // Ignore client-supplied correlation identifiers; never use credentials or
    // nil IDs as a fallback when the operating system RNG fails.
    let request_id = random_uuid().ok().and_then(|id| RequestId::new(id).ok());
    let mut response = if let Some(id) = request_id {
        tracing::Span::current().record("request_id", field::display(id));
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
