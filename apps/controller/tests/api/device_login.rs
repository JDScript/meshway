use super::*;
use meshway_protocol::{AuthorizeDeviceResponse, DEVICE_CALLBACK_URI, s256};

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

async fn code(f: &Fixture) -> String {
    let user = f.original_user().await;
    let (status, body) = request(&f.app, "POST", "/api/v1/device-authorizations", Some(&user), Some(json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_challenge":s256(VERIFIER),"name":"Mac","agent_version":"v1"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value::<AuthorizeDeviceResponse>(body)
        .unwrap()
        .code
}

fn exchange(credential: &str) -> Value {
    json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_verifier":VERIFIER,"node_credential":credential,"name":"Mac","agent_version":"v1"})
}

#[tokio::test]
async fn device_authorization_requires_user_and_valid_callback() {
    let f = Fixture::new();
    let body = json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_challenge":s256(VERIFIER),"name":"Mac","agent_version":"v1"});
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/device-authorizations",
            Some(&f.admin),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for token in [None, Some("invalid")] {
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/device-authorizations",
                token,
                Some(body.clone())
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let mut wrong = body;
    wrong["redirect_uri"] = json!("https://evil.test");
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/device-authorizations",
            Some(&f.admin),
            Some(wrong)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn device_exchange_binding_concurrency_recovery_and_revocation() {
    let f = Fixture::new();
    let code = code(&f).await;
    let credential = local::credential().unwrap();
    let body = exchange(&credential);
    let ordinary = f.token().await.token;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&ordinary),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // A failed device exchange leaves the ordinary token usable by its own path.
    enroll(&f, &ordinary, &local::credential().unwrap(), "ordinary").await;
    for (key, value) in [
        ("code_verifier", json!("a".repeat(43))),
        ("redirect_uri", json!("https://evil.test")),
        ("name", json!("Other")),
        ("agent_version", json!("v2")),
        ("node_credential", json!("bad")),
    ] {
        let mut wrong = body.clone();
        wrong[key] = value;
        let (status, error) = request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(wrong),
        )
        .await;
        assert_eq!(
            status,
            if key == "redirect_uri" || key == "node_credential" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        for secret in [code.as_str(), credential.as_str(), VERIFIER] {
            assert!(!error.to_string().contains(secret));
        }
    }
    // Codes cannot be redeemed through the ordinary enrollment route.
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/enroll",
            Some(&code),
            Some(enrollment(&credential, "Mac"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (a, b) = tokio::join!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(body.clone())
        ),
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(body.clone())
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1, b.1);
    let node: AgentNodeResponse = serde_json::from_value(a.1).unwrap();
    let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
    db.execute(
        "UPDATE enrollment_tokens SET expires_at=0 WHERE token_hash=?1",
        [local::digest(&code)],
    )
    .unwrap();
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    for (key, value) in [
        ("node_credential", json!(local::credential().unwrap())),
        ("code_verifier", json!("b".repeat(43))),
        ("name", json!("Other")),
        ("agent_version", json!("v2")),
    ] {
        let mut wrong = body.clone();
        wrong[key] = value;
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/devices/exchange",
                Some(&code),
                Some(wrong)
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    request(
        &f.app,
        "POST",
        "/api/v1/nodes/revoke",
        Some(&f.admin),
        Some(json!({"node_id":node.node.id})),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(body)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let expired = self::code(&f).await;
    db.execute(
        "UPDATE enrollment_tokens SET expires_at=0 WHERE token_hash=?1",
        [local::digest(&expired)],
    )
    .unwrap();
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&expired),
            Some(exchange(&local::credential().unwrap()))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        db.query_row(
            "SELECT used_node_id IS NULL FROM enrollment_tokens WHERE token_hash=?1",
            [local::digest(&expired)],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}
