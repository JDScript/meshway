use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use meshway_controller::{AppState, admin_token, openapi_json, router};
use meshway_protocol::{
    AgentNodeResponse, CreateEnrollmentTokenResponse, NodeList, RevokeNodeResponse, local,
};
use serde_json::{Value, json};
use tower::ServiceExt;

#[path = "api/access.rs"]
mod access;
#[path = "api/accounts.rs"]
mod accounts;
#[path = "api/connection_runtime.rs"]
mod connection_runtime;
#[path = "api/device_login.rs"]
mod device_login;
#[path = "api/endpoint_discovery.rs"]
mod endpoint_discovery;
#[path = "api/network.rs"]
mod network;
#[path = "api/network_endpoint_matrix.rs"]
mod network_endpoint_matrix;
#[path = "api/networks.rs"]
mod networks;

fn tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

struct Fixture {
    directory: tempfile::TempDir,
    app: Router,
    admin: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempdir();
        let app = router(AppState::open(directory.path()).unwrap());
        let admin = admin_token(directory.path()).unwrap();
        Self {
            directory,
            app,
            admin,
        }
    }

    async fn token(&self) -> CreateEnrollmentTokenResponse {
        let (status, body) = request(
            &self.app,
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&self.admin),
            Some(json!({"expires_in_seconds": 3600})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_value(body).unwrap()
    }

    async fn user_for(&self, id: &str) -> String {
        let email = format!("{id}@fixture.test");
        let (status, body) = request(
            &self.app,
            "POST",
            &format!("/api/v1/networks/{id}/owner-claim"),
            Some(&self.admin),
            Some(json!({"kind":"new_account","email":email,"password":"fixture password secret"})),
        )
        .await;
        assert!(
            status == StatusCode::OK || status == StatusCode::CONFLICT,
            "{body}"
        );
        let (status, body) = request(
            &self.app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({"email":email,"password":"fixture password secret"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["session_token"].as_str().unwrap().into()
    }
    async fn original_user(&self) -> String {
        let (_, networks) = request(
            &self.app,
            "GET",
            "/api/v1/networks",
            Some(&self.admin),
            None,
        )
        .await;
        self.user_for(networks["networks"][0]["network_id"].as_str().unwrap())
            .await
    }
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let path = scoped_path(app, path, token).await;
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = if let Some(body) = body {
        request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
    } else {
        request.body(Body::empty())
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

// Existing single-network behavior cases target the original network through
// the actual scoped routes. Multi-network tests supply explicit paths.
fn contract_path(path: &str) -> String {
    if is_admin_path(path) {
        format!(
            "/api/v1/networks/{{network_id}}/{}",
            path.trim_start_matches("/api/v1/")
        )
    } else {
        path.to_owned()
    }
}

fn is_admin_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap();
    [
        "/api/v1/overview",
        "/api/v1/network",
        "/api/v1/device-authorizations",
        "/api/v1/enrollment-tokens",
        "/api/v1/access",
        "/api/v1/tags",
        "/api/v1/flows/preview",
    ]
    .contains(&path)
        || path.starts_with("/api/v1/nodes")
        || path.starts_with("/api/v1/policy")
        || path.starts_with("/api/v1/connections")
}

async fn scoped_path(app: &Router, path: &str, token: Option<&str>) -> String {
    if !is_admin_path(path) {
        return path.to_owned();
    }
    let mut lookup = Request::get("/api/v1/networks");
    if let Some(token) = token {
        lookup = lookup.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(lookup.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    let id = list["networks"][0]["network_id"]
        .as_str()
        .unwrap_or("00000000000000000000000000000000");
    format!(
        "/api/v1/networks/{id}/{}",
        path.trim_start_matches("/api/v1/")
    )
}

fn enrollment(credential: &str, name: &str) -> Value {
    json!({"node_credential": credential, "name": name, "agent_version": "0.1.0"})
}

async fn enroll(fixture: &Fixture, token: &str, credential: &str, name: &str) -> AgentNodeResponse {
    let (status, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/agents/enroll",
        Some(token),
        Some(enrollment(credential, name)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn credentials_are_scoped_and_heartbeat_identity_is_server_derived() {
    let fixture = Fixture::new();
    let first_token = fixture.token().await.token;
    let first_credential = local::credential().unwrap();
    let first = enroll(&fixture, &first_token, &first_credential, "first")
        .await
        .node;
    let second_token = fixture.token().await.token;
    let second_credential = local::credential().unwrap();
    let second = enroll(&fixture, &second_token, &second_credential, "second")
        .await
        .node;
    assert_ne!(first.id, second.id);
    assert_eq!(first.registered_at, first.last_seen);
    assert!(first.revoked_at.is_none());
    for token in [
        None,
        Some("invalid"),
        Some(first_token.as_str()),
        Some(first_credential.as_str()),
    ] {
        for path in ["/api/v1/overview", "/api/v1/nodes"] {
            assert_eq!(
                request(&fixture.app, "GET", path, token, None).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        for (path, body) in [
            (
                "/api/v1/enrollment-tokens",
                json!({"expires_in_seconds":60}),
            ),
            ("/api/v1/nodes/revoke", json!({"node_id":first.id})),
        ] {
            assert_eq!(
                request(&fixture.app, "POST", path, token, Some(body))
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    for token in [
        None,
        Some("invalid"),
        Some(fixture.admin.as_str()),
        Some(first_credential.as_str()),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agents/enroll",
                token,
                Some(enrollment(&local::credential().unwrap(), "x"))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for token in [
        None,
        Some("invalid"),
        Some(fixture.admin.as_str()),
        Some(first_token.as_str()),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agents/heartbeat",
                token,
                Some(json!({"agent_version":"0.2.0"}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/heartbeat",
            Some(&first_credential),
            Some(json!({"agent_version":"0.2.0", "node_id":second.id}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/agents/heartbeat",
        Some(&first_credential),
        Some(json!({"agent_version":"0.2.0"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["node"]["id"], first.id);
    let (_, list) = request(
        &fixture.app,
        "GET",
        "/api/v1/nodes",
        Some(&fixture.admin),
        None,
    )
    .await;
    let nodes: NodeList = serde_json::from_value(list).unwrap();
    assert_eq!(
        nodes
            .nodes
            .iter()
            .find(|node| node.id == second.id)
            .unwrap()
            .last_seen,
        second.last_seen
    );
    assert_eq!(
        nodes
            .nodes
            .iter()
            .find(|node| node.id == first.id)
            .unwrap()
            .agent_version,
        "0.2.0"
    );
    assert_eq!(
        request(&fixture.app, "POST", "/api/v1/agents/report", None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn invalid_requests_preserve_token_and_unicode_limits_match_the_contract() {
    let fixture = Fixture::new();
    for ttl in [json!(59), json!(86401), json!(-1), json!(60.5), json!("60")] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/enrollment-tokens",
                Some(&fixture.admin),
                Some(json!({"expires_in_seconds":ttl}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    for body in [
        enrollment(&credential, ""),
        enrollment(&credential, " \t\u{0085}"),
        enrollment(&credential, &"😀".repeat(129)),
        enrollment("x", "x"),
        json!({"node_credential":credential,"name":"x","agent_version":""}),
        json!({"node_credential":credential,"name":"x","agent_version":"0.1.0","node_id":"chosen"}),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agents/enroll",
                Some(&token),
                Some(body)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    // A token itself cannot be promoted into a node credential.
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&token),
            Some(enrollment(&token, "x"))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let node = enroll(&fixture, &token, &credential, &"😀".repeat(128))
        .await
        .node;
    let retry = enroll(&fixture, &token, &credential, &"😀".repeat(128))
        .await
        .node;
    assert_eq!(node.id, retry.id);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&token),
            Some(enrollment(&credential, "different name"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let other_token = fixture.token().await.token;
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&other_token),
            Some(enrollment(&credential, "other"))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    enroll(
        &fixture,
        &other_token,
        &local::credential().unwrap(),
        "other",
    )
    .await;
    let (_, overview) = request(
        &fixture.app,
        "GET",
        "/api/v1/overview",
        Some(&fixture.admin),
        None,
    )
    .await;
    assert_eq!(overview["node_count"], 2);
}

#[tokio::test]
async fn request_media_size_and_json_errors_are_safe_and_do_not_consume_token() {
    let fixture = Fixture::new();
    let token = fixture.token().await.token;
    for (body, media, expected) in [
        ("{".to_owned(), "application/json", 400),
        ("{}".to_owned(), "text/plain", 415),
        (" ".repeat(8193), "application/json", 413),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::post("/api/v1/agents/enroll")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", media)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(&token));
    }
    enroll(&fixture, &token, &local::credential().unwrap(), "valid").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_time_consumption_is_atomic_under_concurrent_distinct_and_bound_retries() {
    let fixture = Fixture::new();
    let token = fixture.token().await.token;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let app = fixture.app.clone();
        let token = token.clone();
        tasks.spawn(async move {
            let credential = local::credential().unwrap();
            let response = request(
                &app,
                "POST",
                "/api/v1/agents/enroll",
                Some(&token),
                Some(enrollment(&credential, "racer")),
            )
            .await;
            (credential, response)
        });
    }
    let mut winner = None;
    while let Some(result) = tasks.join_next().await {
        let (credential, (status, body)) = result.unwrap();
        if status == StatusCode::OK {
            assert!(winner.is_none());
            winner = Some((credential, body["node"]["id"].clone()));
        } else {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }
    let (credential, id) = winner.unwrap();
    for _ in 0..16 {
        let (app, token, credential) = (fixture.app.clone(), token.clone(), credential.clone());
        tasks.spawn(async move {
            (
                credential.clone(),
                request(
                    &app,
                    "POST",
                    "/api/v1/agents/enroll",
                    Some(&token),
                    Some(enrollment(&credential, "racer")),
                )
                .await,
            )
        });
    }
    while let Some(result) = tasks.join_next().await {
        let (_, (status, body)) = result.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["node"]["id"], id);
    }
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            "/api/v1/overview",
            Some(&fixture.admin),
            None
        )
        .await
        .1["node_count"],
        1
    );
}

#[tokio::test]
async fn expiry_restart_and_revocation_preserve_identity_and_consumption() {
    let fixture = Fixture::new();
    let expired = fixture.token().await;
    let active = fixture.token().await;
    let unused = fixture.token().await;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &active.token, &credential, "durable")
        .await
        .node;
    // Set expiry in this isolated test database instead of sleeping for 60s.
    let connection =
        rusqlite::Connection::open(fixture.directory.path().join("controller.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE enrollment_tokens SET expires_at = 0 WHERE id IN (?1, ?2)",
            [&expired.token_id, &active.token_id],
        )
        .unwrap();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&expired.token),
            Some(enrollment(&local::credential().unwrap(), "late"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        enroll(&fixture, &active.token, &credential, "durable")
            .await
            .node
            .id,
        node.id
    );
    drop(connection);
    let Fixture {
        directory,
        app,
        admin,
    } = fixture;
    drop(app);
    let restarted = Fixture {
        app: router(AppState::open(directory.path()).unwrap()),
        directory,
        admin,
    };
    assert_eq!(
        admin_token(restarted.directory.path()).unwrap(),
        restarted.admin
    );
    assert_eq!(
        enroll(&restarted, &active.token, &credential, "durable")
            .await
            .node
            .id,
        node.id
    );
    assert_eq!(
        request(
            &restarted.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&active.token),
            Some(enrollment(&local::credential().unwrap(), "different"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    enroll(
        &restarted,
        &unused.token,
        &local::credential().unwrap(),
        "unused-survived",
    )
    .await;
    let mut revoked_at = None;
    for _ in 0..2 {
        let (status, body) = request(
            &restarted.app,
            "POST",
            "/api/v1/nodes/revoke",
            Some(&restarted.admin),
            Some(json!({"node_id":node.id})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let response: RevokeNodeResponse = serde_json::from_value(body).unwrap();
        assert!(response.node.revoked_at.is_some());
        if let Some(time) = revoked_at {
            assert_eq!(response.node.revoked_at, Some(time));
        }
        revoked_at = response.node.revoked_at;
    }
    let Fixture {
        directory,
        app,
        admin,
    } = restarted;
    drop(app);
    let restarted = Fixture {
        app: router(AppState::open(directory.path()).unwrap()),
        directory,
        admin,
    };
    assert_eq!(
        request(
            &restarted.app,
            "POST",
            "/api/v1/agents/heartbeat",
            Some(&credential),
            Some(json!({"agent_version":"0.2.0"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &restarted.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&active.token),
            Some(enrollment(&credential, "durable"))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let fresh = restarted.token().await.token;
    assert_eq!(
        request(
            &restarted.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&fresh),
            Some(enrollment(&credential, "durable"))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, list) = request(
        &restarted.app,
        "GET",
        "/api/v1/nodes",
        Some(&restarted.admin),
        None,
    )
    .await;
    let list: NodeList = serde_json::from_value(list).unwrap();
    let stored = list
        .nodes
        .iter()
        .find(|stored| stored.id == node.id)
        .unwrap();
    assert_eq!(stored.revoked_at, revoked_at);
    assert_eq!(stored.last_seen, node.last_seen);
    assert_eq!(
        request(
            &restarted.app,
            "GET",
            "/api/v1/overview",
            Some(&restarted.admin),
            None
        )
        .await
        .1["node_count"],
        2
    );
    assert_eq!(
        request(
            &restarted.app,
            "POST",
            "/api/v1/nodes/revoke",
            Some(&restarted.admin),
            Some(json!({"node_id":"missing"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn private_bootstrap_rejects_partial_mismatched_and_concurrently_owned_state() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let directory = tempdir();
    assert!(admin_token(directory.path()).is_err());
    let state = AppState::open(directory.path()).unwrap();
    let admin = admin_token(directory.path()).unwrap();
    assert!(AppState::open(directory.path()).is_err());
    for file in ["admin.token", "controller.sqlite3", "controller.lock"] {
        assert_eq!(
            std::fs::metadata(directory.path().join(file))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(
        std::fs::metadata(directory.path()).unwrap().mode() & 0o777,
        0o700
    );
    let bytes = std::fs::read(directory.path().join("controller.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(admin.len())
            .any(|window| window == admin.as_bytes())
    );
    drop(state);
    let token_file = directory.path().join("admin.token");
    std::fs::write(&token_file, local::credential().unwrap()).unwrap();
    assert!(AppState::open(directory.path()).is_err());
    assert!(admin_token(directory.path()).is_err());
    std::fs::write(&token_file, &admin).unwrap();
    std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(AppState::open(directory.path()).is_err());
    std::fs::remove_file(&token_file).unwrap();
    assert!(AppState::open(directory.path()).is_err());
    let other = directory.path().join("other.token");
    local::atomic_write(&other, admin.as_bytes(), false).unwrap();
    symlink(&other, &token_file).unwrap();
    assert!(AppState::open(directory.path()).is_err());
    std::fs::remove_file(&token_file).unwrap();
    std::fs::remove_file(directory.path().join("controller.sqlite3")).unwrap();
    assert!(
        AppState::open(directory.path()).is_err(),
        "unknown files must not be reinitialized"
    );
}

#[test]
fn committed_openapi_matches_live_routes_and_current_version() {
    assert_eq!(
        openapi_json().unwrap(),
        include_str!("../../../packages/protocol/openapi.json"),
        "Run pnpm --filter @meshway/control-client generate"
    );
}

#[tokio::test]
async fn policy_metadata_and_configuration_routes_enforce_roles_and_conditional_writes() {
    let fixture = Fixture::new();
    let enrollment_token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &enrollment_token, &credential, "policy-source")
        .await
        .node;
    let policy = json!({"default_action":"direct","rules":[]});
    let flow = json!({"source_node_id":node.id,"destination_node_id":null,"destination_ip":null,"domain":"example.com","destination_country":null});
    for wrong in [&credential, &enrollment_token] {
        assert_eq!(
            request(&fixture.app, "GET", "/api/v1/policy", Some(wrong), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        for (path, body) in [
            (
                "/api/v1/policy/preview",
                json!({"policy":policy,"flow":flow}),
            ),
            (
                "/api/v1/policy/save",
                json!({"expected_revision":2,"policy":policy}),
            ),
            (
                "/api/v1/nodes/country",
                json!({"node_id":node.id,"country":"us","expected_revision":2}),
            ),
        ] {
            assert_eq!(
                request(&fixture.app, "POST", path, Some(wrong), Some(body))
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    for wrong in [&fixture.admin, &enrollment_token] {
        assert_eq!(
            request(
                &fixture.app,
                "GET",
                "/api/v1/agents/configuration",
                Some(wrong),
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agents/configuration/ack",
                Some(wrong),
                Some(json!({"revision":2,"digest":"0".repeat(64)}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (code, preview) = request(
        &fixture.app,
        "POST",
        "/api/v1/policy/preview",
        Some(&fixture.admin),
        Some(json!({"policy":policy,"flow":flow})),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(preview["revision"], 2);
    assert_eq!(preview["decision"]["action"], "route");
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            "/api/v1/policy",
            Some(&fixture.admin),
            None
        )
        .await
        .1["policy"]["default_action"],
        "reject"
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/policy/save",
            Some(&fixture.admin),
            Some(json!({"expected_revision":1,"policy":policy}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let invalid = json!({"default_action":"direct","rules":[{"id":"invalid","source":{"node_ids":[],"countries":[]},"destination_scope":"mesh","destination_any":[],"action":{"kind":"route","via":["missing"],"exit_node_id":null}}]});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/policy/save",
            Some(&fixture.admin),
            Some(json!({"expected_revision":2,"policy":invalid}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            "/api/v1/policy",
            Some(&fixture.admin),
            None
        )
        .await
        .1["revision"],
        2
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/country",
            Some(&fixture.admin),
            Some(json!({"node_id":node.id,"expected_revision":2}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "country is required even when null"
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/country",
            Some(&fixture.admin),
            Some(json!({"node_id":node.id,"country":"USA","expected_revision":2}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let updated = request(
        &fixture.app,
        "POST",
        "/api/v1/nodes/country",
        Some(&fixture.admin),
        Some(json!({"node_id":node.id,"country":"us","expected_revision":2})),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["node"]["country"], "US");
    let snapshot = request(
        &fixture.app,
        "GET",
        "/api/v1/agents/configuration?node_id=someone-else",
        Some(&credential),
        None,
    )
    .await;
    assert_eq!(snapshot.0, StatusCode::OK);
    assert_eq!(snapshot.1["payload"]["node"]["id"], node.id);
    assert_eq!(snapshot.1["payload"]["revision"], 3);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(&credential),
            Some(json!({"revision":2,"digest":snapshot.1["digest"]}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(&credential),
            Some(json!({"revision":3,"digest":snapshot.1["digest"]}))
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn connection_routes_scope_identity_roles_strict_bodies_and_saved_only_receipts() {
    let fixture = Fixture::new();
    let token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &token, &credential, "connections")
        .await
        .node;
    let identity = json!({"public_key":format!("09{}","00".repeat(31)),"capabilities":[{"transport":"native_tcp","can_dial":true,"can_accept":true,"semantics":"stream"}]});
    let empty = json!({"expected_revision":2,"configuration":{"listeners":[],"candidates":[]}});
    let ack = json!({"revision":2,"digest":"00".repeat(32)});
    for wrong in [&credential, &token] {
        assert_eq!(
            request(
                &fixture.app,
                "GET",
                "/api/v1/connections",
                Some(wrong),
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/connections",
                Some(wrong),
                Some(empty.clone())
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for wrong in [&fixture.admin, &token] {
        assert_eq!(
            request(
                &fixture.app,
                "GET",
                "/api/v1/agent/connections",
                Some(wrong),
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agent/connection-identity",
                Some(wrong),
                Some(identity.clone())
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agent/connections/ack",
                Some(wrong),
                Some(ack.clone())
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for bad in [
        json!({"public_key":identity["public_key"],"capabilities":[],"node_id":node.id}),
        json!({"public_key":"00".repeat(32),"capabilities":[]}),
        json!({"public_key":"AB".repeat(32),"capabilities":[]}),
        json!({"public_key":identity["public_key"],"capabilities":[{"transport":"wireguard","can_dial":true,"can_accept":true,"semantics":"datagram"}]}),
        json!({"public_key":identity["public_key"],"capabilities":[{"transport":"native_udp","can_dial":true,"can_accept":true,"semantics":"stream"}]}),
        json!({"public_key":identity["public_key"],"capabilities":[{"transport":"native_tcp","can_dial":"true","can_accept":true,"semantics":"stream"}]}),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agent/connection-identity",
                Some(&credential),
                Some(bad)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let published = request(
        &fixture.app,
        "POST",
        "/api/v1/agent/connection-identity?node_id=other",
        Some(&credential),
        Some(identity.clone()),
    )
    .await;
    assert_eq!(published.0, StatusCode::OK);
    assert_eq!(published.1["node_id"], node.id);
    assert_eq!(published.1["revision"], 3);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/connections",
            Some(&fixture.admin),
            Some(empty)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let snapshot = request(
        &fixture.app,
        "GET",
        "/api/v1/agent/connections?node_id=other",
        Some(&credential),
        None,
    )
    .await;
    assert_eq!(snapshot.1["payload"]["node_id"], node.id);
    let receipt = json!({"revision":3,"digest":snapshot.1["digest"]});
    let saved = request(
        &fixture.app,
        "POST",
        "/api/v1/agent/connections/ack",
        Some(&credential),
        Some(receipt.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK);
    assert_eq!(saved.1["status"], "saved");
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            "/api/v1/nodes",
            Some(&fixture.admin),
            None
        )
        .await
        .1["nodes"][0]["acknowledged_revision"],
        0
    );
    for bad in [
        json!({"revision":"3","digest":snapshot.1["digest"]}),
        json!({"revision":3,"digest":"bad"}),
        json!({"revision":0,"digest":snapshot.1["digest"]}),
        json!({"revision":3,"digest":snapshot.1["digest"],"node_id":node.id}),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agent/connections/ack",
                Some(&credential),
                Some(bad)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    request(
        &fixture.app,
        "POST",
        "/api/v1/nodes/revoke",
        Some(&fixture.admin),
        Some(json!({"node_id":node.id})),
    )
    .await;
    for (method, path, body) in [
        ("GET", "/api/v1/agent/connections", None),
        ("POST", "/api/v1/agent/connection-identity", Some(identity)),
        ("POST", "/api/v1/agent/connections/ack", Some(receipt)),
    ] {
        assert_eq!(
            request(&fixture.app, method, path, Some(&credential), body)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn connections_concurrent_admin_cas_has_one_winner_and_scoped_body_budget() {
    let fixture = Fixture::new();
    let token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &token, &credential, "listener").await.node;
    assert_eq!(request(&fixture.app,"POST","/api/v1/agent/connection-identity",Some(&credential),Some(json!({"public_key":format!("09{}","00".repeat(31)),"capabilities":[{"transport":"native_tcp","can_dial":false,"can_accept":true,"semantics":"stream"}]}))).await.0,StatusCode::OK);
    let plan = |port| json!({"expected_revision":3,"configuration":{"listeners":[{"id":"listener","node_id":node.id,"transport":"native_tcp","bind":format!("0.0.0.0:{port}"),"advertised_endpoints":[{"host":"127.0.0.1","port":port}],"credential_ref":null}],"candidates":[]}});
    let (one, two) = tokio::join!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/connections",
            Some(&fixture.admin),
            Some(plan(8443))
        ),
        request(
            &fixture.app,
            "POST",
            "/api/v1/connections",
            Some(&fixture.admin),
            Some(plan(8444))
        )
    );
    assert!(
        (one.0 == StatusCode::OK && two.0 == StatusCode::CONFLICT)
            || (two.0 == StatusCode::OK && one.0 == StatusCode::CONFLICT)
    );
    let winner = if one.0 == StatusCode::OK {
        one.1
    } else {
        two.1
    };
    assert_eq!(winner["revision"], 4);
    for (path, token, body, padding, expected) in [
        (
            "/api/v1/connections",
            fixture.admin.as_str(),
            json!({"expected_revision":4,"configuration":winner["configuration"]}),
            9000,
            StatusCode::OK,
        ),
        (
            "/api/v1/connections",
            fixture.admin.as_str(),
            json!({"expected_revision":4,"configuration":winner["configuration"]}),
            128 * 1024,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "/api/v1/agent/connection-identity",
            credential.as_str(),
            json!({"public_key":format!("09{}","00".repeat(31)),"capabilities":[]}),
            9000,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "/api/v1/agent/connections/ack",
            credential.as_str(),
            json!({"revision":4,"digest":"00".repeat(32)}),
            9000,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "/api/v1/enrollment-tokens",
            fixture.admin.as_str(),
            json!({"expires_in_seconds":60}),
            9000,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(scoped_path(&fixture.app, path, Some(token)).await)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(format!("{}{body}", " ".repeat(padding))))
            .unwrap();
        assert_eq!(
            fixture.app.clone().oneshot(request).await.unwrap().status(),
            expected,
            "{path}"
        );
    }
    let mut missing_ref = plan(8443);
    missing_ref["expected_revision"] = 4.into();
    missing_ref["configuration"]["listeners"][0]
        .as_object_mut()
        .unwrap()
        .remove("credential_ref");
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/connections",
            Some(&fixture.admin),
            Some(missing_ref)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = plan(8443);
    invalid["expected_revision"] = 4.into();
    invalid["configuration"]["listeners"][0]["advertised_endpoints"][0]["port"] = 0.into();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/connections",
            Some(&fixture.admin),
            Some(invalid)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for port in [json!(65536), json!(-1), json!(1.5), json!("443")] {
        let mut invalid = plan(8443);
        invalid["expected_revision"] = 4.into();
        invalid["configuration"]["listeners"][0]["advertised_endpoints"][0]["port"] = port;
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/connections",
                Some(&fixture.admin),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}
