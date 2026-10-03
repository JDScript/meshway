#![cfg(unix)]

use std::{path::Path, time::Duration};

use meshway_controller::{App, Settings, http};
use meshway_protocol::{
    AuthOptionsResponse, AuthSessionResponse, ErrorResponse, LoginRequest, PasswordLengthUnit,
    Problem, RegisterRequest, SessionIssuedResponse, WireSecret,
};
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
        let app = App::open(settings).await.expect("open real SQLite");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = http::router(app.accounts());
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
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}

fn client() -> Client {
    Client::builder()
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
        error(
            client
                .post(server.auth("register"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    error(
        client
            .post(server.auth("register"))
            .body("sensitive body")
            .send()
            .await
            .unwrap(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
    error(
        client
            .post(server.auth("login"))
            .header(header::CONTENT_TYPE, "application/json")
            .body("sensitive".repeat(http::MAX_JSON_BODY_BYTES))
            .send()
            .await
            .unwrap(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_too_large",
    )
    .await;
    error(
        client
            .post(server.auth("register"))
            .json(&RegisterRequest {
                email: "not an email".into(),
                password: WireSecret::new("sensitive password".into()),
            })
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await;

    // Six supplementary-plane scalars are twelve UTF-16 units, but still weak.
    let weak = error(
        client
            .post(server.auth("register"))
            .json(&RegisterRequest {
                email: "short@example.com".into(),
                password: WireSecret::new("😀".repeat(6)),
            })
            .send()
            .await
            .unwrap(),
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
    error(
        client
            .post(server.auth("register"))
            .json(&RegisterRequest {
                email: alice.user.email.clone(),
                password: WireSecret::new(password.clone()),
            })
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "email_taken",
    )
    .await;
    error(
        client
            .post(server.auth("login"))
            .json(&LoginRequest {
                email: alice.user.email.clone(),
                password: WireSecret::new("sensitive wrong password".into()),
            })
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    )
    .await;
    error(
        client
            .post(server.auth("login"))
            .json(&LoginRequest {
                email: "unknown@local".into(),
                password: WireSecret::new("sensitive wrong password".into()),
            })
            .send()
            .await
            .unwrap(),
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
    error(
        client.post(server.auth("logout")).send().await.unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    error(
        client
            .get(format!("{}/api/v1/networks", server.url))
            .bearer_auth(alice.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    error(
        client
            .get(format!("{}/agent-api/v1/auth/session", server.url))
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
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
    let options: AuthOptionsResponse = client
        .get(server.auth("options"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!options.registration_enabled);
    assert_eq!(
        session(&client, &server, alice.token.expose())
            .await
            .session,
        alice.session
    );
    error(
        client
            .post(server.auth("register"))
            .json(&RegisterRequest {
                email: "new@local".into(),
                password: WireSecret::new(password.clone()),
            })
            .send()
            .await
            .unwrap(),
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
    error(
        client
            .post(server.auth("logout"))
            .bearer_auth(alice.token.expose())
            .send()
            .await
            .unwrap(),
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
    error(
        client
            .get(server.auth("session"))
            .bearer_auth(alice.token.expose())
            .send()
            .await
            .unwrap(),
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
    error(
        client
            .get(format!("{url}/api/v1/auth/session"))
            .bearer_auth(login.token.expose())
            .send()
            .await
            .unwrap(),
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
    assert_eq!(spec, artifact);
    assert_eq!(spec["openapi"], "3.1.0");
    let paths = spec["paths"].as_object().unwrap();
    assert_eq!(paths.len(), 5);
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
}
