#![cfg(unix)]
mod feedback;
mod live;
mod management;
mod preview;
static ACCOUNT_FLOW: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use std::{path::Path, time::Duration};

use meshway_protocol::{
    AuthOptionsResponse, AuthSessionResponse, ErrorResponse, LoginRequest, PasswordLengthUnit,
    Problem, RegisterRequest, SessionIssuedResponse, WireSecret,
};
use meshway_server::{App, Settings, http};
use reqwest::{Client, Response, StatusCode, header};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

struct Server {
    app: App,
    url: String,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(path: &Path, registration: bool) -> Self {
        let mut settings = Settings::new(path);
        settings.registration_enabled = registration;
        Self::start_with(settings).await
    }
    async fn start_with(settings: Settings) -> Self {
        let app = App::open(settings).await.expect("open real SQLite");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = http::router(&app);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            app,
            url,
            stop,
            task,
        }
    }

    fn auth(&self, operation: &str) -> String {
        format!("{}/api/v1/auth/{operation}", self.url)
    }

    async fn close(self) {
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}

fn client() -> Client {
    Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn assert_headers(response: &Response) {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .parse::<meshway_model::RequestId>()
            .is_ok()
    );
}

async fn error(response: Response, status: StatusCode, code: &str) -> ErrorResponse {
    assert_eq!(response.status(), status);
    assert_headers(&response);
    if status == StatusCode::UNAUTHORIZED {
        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            "Bearer realm=\"meshway\""
        );
    }
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let text = response.text().await.unwrap();
    assert!(!text.contains("sensitive"));
    assert!(!text.contains("mws_"));
    let error: ErrorResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(error.status, status.as_u16());
    assert_eq!(error.request_id.unwrap().to_string(), request_id);
    assert_eq!(serde_json::to_value(&error.problem).unwrap()["code"], code);
    error
}

async fn json<T: serde::de::DeserializeOwned>(request: reqwest::RequestBuilder) -> T {
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("{error}: {body}"))
}

async fn reject(request: reqwest::RequestBuilder, status: StatusCode, code: &str) -> ErrorResponse {
    error(request.send().await.unwrap(), status, code).await
}

async fn empty(request: reqwest::RequestBuilder) {
    assert_eq!(
        request.send().await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

async fn register(
    client: &Client,
    server: &Server,
    email: &str,
    password: &str,
) -> SessionIssuedResponse {
    let response = client
        .post(server.auth("register"))
        .json(&RegisterRequest {
            email: email.into(),
            password: WireSecret::new(password.into()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_headers(&response);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let issued: SessionIssuedResponse = response.json().await.unwrap();
    assert_eq!(issued.request_id.to_string(), request_id);
    assert!(issued.token.expose().starts_with("mws_"));
    assert!(issued.session.expires_at > issued.session.created_at);
    issued
}

async fn session(client: &Client, server: &Server, token: &str) -> AuthSessionResponse {
    let response = client
        .get(server.auth("session"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_headers(&response);
    let text = response.text().await.unwrap();
    assert!(!text.contains(token));
    assert!(!text.contains("\"token\""));
    serde_json::from_str(&text).unwrap()
}

// One sequential account scenario: production Argon2 budget, never a test KDF.
#[tokio::test]
async fn real_accounts_http_persistence_and_safe_rejections() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let server = Server::start(&path, true).await;
    let client = client();
    let response = client.get(server.auth("options")).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_headers(&response);
    let options: AuthOptionsResponse = response.json().await.unwrap();
    assert!(options.registration_enabled);
    assert_eq!(options.password_rules.min_codepoints, 12);
    assert_eq!(options.password_rules.max_codepoints, 128);
    assert_eq!(options.password_rules.max_utf8_bytes, 512);
    assert_eq!(
        options.password_rules.length_unit,
        PasswordLengthUnit::UnicodeCodepoints
    );

    for body in [
        r#"["sensitive@example.com","sensitive password"]"#,
        r#"{"email":"sensitive@example.com","password":"sensitive password","extra":true}"#,
        r#"{"email":"sensitive@example.com","password":123}"#,
        r#"{"email":"sensitive@example.com","password":"sensitive password","password":"duplicate"}"#,
        r#"{"email":"sensitive""#,
    ] {
        reject(
            client
                .post(server.auth("register"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    reject(
        client.post(server.auth("register")).body("sensitive body"),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
    reject(
        client
            .post(server.auth("login"))
            .header(header::CONTENT_TYPE, "application/json")
            .body("sensitive".repeat(http::MAX_JSON_BODY_BYTES)),
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_too_large",
    )
    .await;
    reject(
        client.post(server.auth("register")).json(&RegisterRequest {
            email: "not an email".into(),
            password: WireSecret::new("sensitive password".into()),
        }),
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await;

    // Six supplementary-plane scalars are twelve UTF-16 units, but still weak.
    let weak = reject(
        client.post(server.auth("register")).json(&RegisterRequest {
            email: "short@example.com".into(),
            password: WireSecret::new("😀".repeat(6)),
        }),
        StatusCode::UNPROCESSABLE_ENTITY,
        "password_weak",
    )
    .await;
    assert!(
        matches!(weak.problem, Problem::PasswordWeak { params, .. } if params == options.password_rules)
    );

    let password = "😀".repeat(12);
    let alice = register(&client, &server, "  Alice@bücher.example  ", &password).await;
    assert_eq!(alice.user.email, "alice@xn--bcher-kva.example");
    let bob = register(&client, &server, "bob@local", "sensitive bob password").await;
    assert_ne!(alice.user.id, bob.user.id);
    assert_eq!(
        session(&client, &server, alice.token.expose()).await.user,
        alice.user
    );
    assert_eq!(
        session(&client, &server, bob.token.expose()).await.user,
        bob.user
    );
    reject(
        client.post(server.auth("register")).json(&RegisterRequest {
            email: alice.user.email.clone(),
            password: WireSecret::new(password.clone()),
        }),
        StatusCode::CONFLICT,
        "email_taken",
    )
    .await;
    reject(
        client.post(server.auth("login")).json(&LoginRequest {
            email: alice.user.email.clone(),
            password: WireSecret::new("sensitive wrong password".into()),
        }),
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    )
    .await;
    reject(
        client.post(server.auth("login")).json(&LoginRequest {
            email: "unknown@local".into(),
            password: WireSecret::new("sensitive wrong password".into()),
        }),
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    )
    .await;

    for request in [
        client.get(server.auth("session")),
        client
            .get(server.auth("session"))
            .header(header::AUTHORIZATION, "Basic sensitive"),
        client
            .get(server.auth("session"))
            .header(header::AUTHORIZATION, "Bearer sensitive"),
        client.get(server.auth("session")).header(
            header::AUTHORIZATION,
            format!("Bearer  {}", alice.token.expose()),
        ),
        client.get(server.auth("session")).header(
            header::AUTHORIZATION,
            format!(
                "Bearer {}, Bearer {}",
                alice.token.expose(),
                bob.token.expose()
            ),
        ),
        client
            .get(server.auth("session"))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", alice.token.expose()),
            )
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", bob.token.expose()),
            ),
        client.get(format!(
            "{}?token={}",
            server.auth("session"),
            alice.token.expose()
        )),
    ] {
        error(
            request.send().await.unwrap(),
            StatusCode::UNAUTHORIZED,
            "invalid_session",
        )
        .await;
    }
    reject(
        client.post(server.auth("logout")),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    reject(
        client
            .get(format!("{}/api/v1/not-an-operation", server.url))
            .bearer_auth(alice.token.expose()),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    reject(
        client.get(format!("{}/agent-api/v1/auth/session", server.url)),
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
    error(
        client.delete(server.auth("session")).send().await.unwrap(),
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
    server.close().await;

    let server = Server::start(&path, false).await;
    let options: AuthOptionsResponse = json(client.get(server.auth("options"))).await;
    assert!(!options.registration_enabled);
    assert_eq!(
        session(&client, &server, alice.token.expose())
            .await
            .session,
        alice.session
    );
    reject(
        client.post(server.auth("register")).json(&RegisterRequest {
            email: "new@local".into(),
            password: WireSecret::new(password.clone()),
        }),
        StatusCode::FORBIDDEN,
        "registration_disabled",
    )
    .await;
    let response = client
        .post(server.auth("login"))
        .json(&LoginRequest {
            email: alice.user.email.clone(),
            password: WireSecret::new(password),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let login: SessionIssuedResponse = response.json().await.unwrap();
    assert_eq!(login.user, alice.user);
    assert_ne!(login.session.id, alice.session.id);
    assert_ne!(login.token.expose(), alice.token.expose());
    let logout = client
        .post(server.auth("logout"))
        .bearer_auth(alice.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_headers(&logout);
    assert!(logout.bytes().await.unwrap().is_empty());
    reject(
        client
            .post(server.auth("logout"))
            .bearer_auth(alice.token.expose()),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    assert_eq!(
        session(&client, &server, login.token.expose()).await.user,
        alice.user
    );
    assert_eq!(
        session(&client, &server, bob.token.expose()).await.user,
        bob.user
    );
    server.close().await;

    let server = Server::start(&path, false).await;
    reject(
        client
            .get(server.auth("session"))
            .bearer_auth(alice.token.expose()),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    assert_eq!(
        session(&client, &server, login.token.expose()).await.user,
        alice.user
    );
    assert_eq!(
        session(&client, &server, bob.token.expose()).await.user,
        bob.user
    );
    // A retained router handle observes real service shutdown, not fixture data.
    let Server {
        app,
        url,
        stop,
        task,
    } = server;
    app.shutdown().await.unwrap();
    reject(
        client
            .get(format!("{url}/api/v1/auth/session"))
            .bearer_auth(login.token.expose()),
        StatusCode::SERVICE_UNAVAILABLE,
        "service_unavailable",
    )
    .await;
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[test]
fn exported_openapi_is_the_actual_router_contract() {
    let spec = serde_json::to_value(http::openapi()).unwrap();
    let artifact: serde_json::Value =
        serde_json::from_str(include_str!("../../../../packages/protocol/openapi.json")).unwrap();
    assert!(
        spec == artifact,
        "OpenAPI drift: regenerate using export-openapi"
    );
    assert_eq!(spec["openapi"], "3.1.0");
    let paths = spec["paths"].as_object().unwrap();
    assert_eq!(paths.len(), 98);
    assert_eq!(
        paths["/agent-api/v1/leave"]["post"]["operationId"],
        "leaveNode"
    );
    for (path, method, operation, status, component) in [
        (
            "options",
            "get",
            "getAuthOptions",
            "200",
            "AuthOptionsResponse",
        ),
        (
            "register",
            "post",
            "register",
            "201",
            "SessionIssuedResponse",
        ),
        ("login", "post", "login", "200", "SessionIssuedResponse"),
        (
            "session",
            "get",
            "getAuthSession",
            "200",
            "AuthSessionResponse",
        ),
        ("logout", "post", "logout", "204", ""),
    ] {
        let operation_spec = &paths[&format!("/api/v1/auth/{path}")][method];
        assert_eq!(operation_spec["operationId"], operation);
        let response = &operation_spec["responses"][status];
        if component.is_empty() {
            assert!(response.get("content").is_none());
        } else {
            assert_eq!(
                response["content"]["application/json"]["schema"]["$ref"],
                format!("#/components/schemas/{component}")
            );
        }
        assert_eq!(
            operation_spec["responses"]["503"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ErrorResponse"
        );
    }
    for name in ["RegisterRequest", "LoginRequest"] {
        let schema = &spec["components"]["schemas"][name];
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"]["email"].get("format").is_none());
        assert!(schema["properties"]["password"].get("minLength").is_none());
    }
    for variant in spec["components"]["schemas"]["Problem"]["oneOf"]
        .as_array()
        .unwrap()
    {
        assert_eq!(variant["additionalProperties"], false);
    }
    for name in [
        "ConfigurationRequest",
        "JoinLinkCreated",
        "ActivityActor",
        "ActivityTarget",
    ] {
        for variant in spec["components"]["schemas"][name]["oneOf"]
            .as_array()
            .unwrap()
        {
            assert_eq!(variant["additionalProperties"], false, "{name}");
        }
    }
    for (component, fields) in [
        (
            "PasswordRules",
            &["minCodepoints", "maxCodepoints", "maxUtf8Bytes"][..],
        ),
        ("ErrorResponse", &["status"][..]),
    ] {
        for field in fields {
            let schema = &spec["components"]["schemas"][component]["properties"][field];
            assert_eq!(schema["type"], "integer");
            assert_eq!(schema["minimum"].as_f64(), Some(0.0));
            assert_eq!(schema["maximum"].as_f64(), Some(65535.0));
        }
    }
}

/// Every error response declares the problem codes it can carry, as a
/// comma-separated list in its description; each must be a code the Server's
/// error mapping produces with that status, and a known ProblemCode value.
#[test]
fn error_responses_declare_consistent_problem_codes() {
    let spec = serde_json::to_value(http::openapi()).unwrap();
    let table = http::problem_samples();
    let codes: std::collections::BTreeSet<String> =
        spec["components"]["schemas"]["Problem"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|variant| {
                variant["properties"]["code"]["enum"][0]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
    let mut declared = 0;
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            for (status, response) in operation["responses"].as_object().unwrap() {
                let is_error = response["content"]["application/json"]["schema"]["$ref"]
                    == "#/components/schemas/ErrorResponse";
                if !is_error {
                    continue;
                }
                let label = format!("{method} {path} {status}");
                let description = response["description"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{label}: error response without problem codes"));
                let status: u16 = status.parse().unwrap();
                for code in description.split(',').map(str::trim) {
                    assert!(codes.contains(code), "{label}: unknown problem code {code}");
                    assert!(
                        table
                            .iter()
                            .any(|sample| sample.status == status && sample.code == code),
                        "{label}: {code} is not produced with this status"
                    );
                    declared += 1;
                }
            }
        }
    }
    assert!(declared > 0);
}
