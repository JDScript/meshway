//! A configured browser client obtains a delegated session through an
//! authorization code with PKCE; the session acts as the user but cannot
//! manage the account, and the client's origin may call the API cross-origin.
use super::*;
use meshway_protocol::{
    AuthSessionResponse, CodeChallengeMethod, CreateDelegationRequest, DelegatedSessionResponse,
    DelegationCreated, ExchangeDelegationRequest, SessionIssuedResponse, WireSecret,
};
use meshway_server::accounts::delegation::{DelegationClients, challenge_of};

const CLIENT_ORIGIN: &str = "http://localhost:4321";

#[tokio::test]
async fn delegated_sessions_are_issued_by_pkce_code_and_cannot_manage_the_account() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(dir.path().canonicalize().unwrap().join("controller"));
    settings.delegation_clients =
        DelegationClients::parse([format!("site={CLIENT_ORIGIN}").as_str()]).unwrap();
    settings.public_origin =
        Some(meshway_server::console_origin("https://api.example.test").unwrap());
    let s = Server::start_with(settings).await;
    let c = client();
    let owner = register(&c, &s, "delegating@example.test", "valid password").await;
    let token = owner.token.expose();
    let verifier = "v".repeat(64);
    let redirect = format!("{CLIENT_ORIGIN}/docs/callback?keep=1");
    let authorize = |client: &str, redirect_uri: &str, challenge: &str, bearer: &str| {
        c.post(format!("{}/api/v1/auth/delegations", s.url))
            .bearer_auth(bearer)
            .json(&CreateDelegationRequest {
                client: client.into(),
                redirect_uri: redirect_uri.into(),
                code_challenge: challenge.into(),
                code_challenge_method: CodeChallengeMethod::S256,
            })
    };
    let challenge = challenge_of(&verifier);
    for (client, redirect_uri, challenge) in [
        ("other", redirect.as_str(), challenge.as_str()),
        ("site", "https://evil.example/cb", challenge.as_str()),
        ("site", "http://localhost:4321/cb#frag", challenge.as_str()),
        ("site", redirect.as_str(), "not-a-challenge"),
    ] {
        error(
            authorize(client, redirect_uri, challenge, token)
                .send()
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    let created = authorize("site", &redirect, &challenge, token)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let issued: DelegationCreated = created.json().await.unwrap();
    let code = issued.code.expose().to_owned();
    assert!(code.starts_with("mwd_"));
    assert!(issued.expires_at <= chrono::Utc::now() + chrono::Duration::seconds(61));

    // A wrong verifier spends the code; the right one no longer works.
    let exchange = |client: &str, code: &str, verifier: &str, redirect_uri: &str| {
        c.post(format!("{}/api/v1/auth/delegations/token", s.url))
            .json(&ExchangeDelegationRequest {
                client: client.into(),
                code: WireSecret::new(code.into()),
                code_verifier: WireSecret::new(verifier.into()),
                redirect_uri: redirect_uri.into(),
            })
    };
    error(
        exchange("site", &code, &"w".repeat(64), &redirect)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_delegation",
    )
    .await;
    error(
        exchange("site", &code, &verifier, &redirect)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_delegation",
    )
    .await;
    let issued: DelegationCreated = json(authorize("site", &redirect, &challenge, token)).await;
    let code = issued.code.expose().to_owned();
    error(
        exchange("site", &code, &verifier, "http://localhost:4321/other")
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_delegation",
    )
    .await;
    let issued: DelegationCreated = json(authorize("site", &redirect, &challenge, token)).await;
    let code = issued.code.expose().to_owned();
    let delegated: DelegatedSessionResponse =
        json(exchange("site", &code, &verifier, &redirect)).await;
    assert_eq!(delegated.client, "site");
    assert_eq!(delegated.user.id, owner.user.id);
    assert_eq!(
        delegated.server.as_deref(),
        Some("https://api.example.test")
    );
    assert_eq!(
        delegated.session.expires_at - delegated.session.created_at,
        chrono::Duration::hours(1)
    );
    // The code is one-time.
    error(
        exchange("site", &code, &verifier, &redirect)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_delegation",
    )
    .await;

    // The delegated session acts as the user.
    let bearer = delegated.token.expose();
    let session: AuthSessionResponse = json(c.get(s.auth("session")).bearer_auth(bearer)).await;
    assert_eq!(session.session.id, delegated.session.id);
    let created = create(&c, &s, bearer, "delegated network").await;
    assert_eq!(
        c.get(net(&s, created.network.id, "members"))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    // But not manage the account, other sessions or further delegations.
    for request in [
        c.post(format!("{}/api/v1/account/password", s.url))
            .json(&ChangePasswordRequest {
                current_password: WireSecret::new("valid password".into()),
                password: WireSecret::new("another valid password".into()),
            }),
        c.post(format!("{}/api/v1/account/email", s.url))
            .json(&ChangeEmailRequest {
                password: WireSecret::new("valid password".into()),
                email: "renamed@example.test".into(),
            }),
        c.post(format!("{}/api/v1/account/sessions/revoke-others", s.url)),
        c.post(format!("{}/api/v1/account/sessions/revoke-all", s.url)),
        c.post(format!(
            "{}/api/v1/account/sessions/{}/revoke",
            s.url, owner.session.id
        )),
        c.post(format!("{}/api/v1/auth/delegations", s.url))
            .json(&CreateDelegationRequest {
                client: "site".into(),
                redirect_uri: redirect.clone(),
                code_challenge: challenge.clone(),
                code_challenge_method: CodeChallengeMethod::S256,
            }),
    ] {
        error(
            request.bearer_auth(bearer).send().await.unwrap(),
            StatusCode::FORBIDDEN,
            "permission_denied",
        )
        .await;
    }
    // The owner sees it in the session list under the client's name and may
    // revoke it; the delegated session may also end itself.
    let sessions: AccountSessionsResponse = json(
        c.get(format!("{}/api/v1/account/sessions", s.url))
            .bearer_auth(token),
    )
    .await;
    assert_eq!(sessions.items.len(), 2);
    let listed = sessions
        .items
        .iter()
        .find(|item| item.id == delegated.session.id)
        .unwrap();
    assert_eq!(listed.delegation_client.as_deref(), Some("site"));
    assert!(!listed.current);
    assert!(
        sessions
            .items
            .iter()
            .any(|item| item.current && item.delegation_client.is_none())
    );
    assert_eq!(
        c.post(s.auth("logout"))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    error(
        c.get(s.auth("session"))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap(),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    // A code does not outlive its authorizing session: sign in elsewhere,
    // authorize, sign out, and the code cannot be exchanged.
    let other: SessionIssuedResponse = json(c.post(s.auth("login")).json(&LoginRequest {
        email: "delegating@example.test".into(),
        password: WireSecret::new("valid password".into()),
    }))
    .await;
    let issued: DelegationCreated = json(authorize(
        "site",
        &redirect,
        &challenge,
        other.token.expose(),
    ))
    .await;
    assert_eq!(
        c.post(s.auth("logout"))
            .bearer_auth(other.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    error(
        exchange("site", issued.code.expose(), &verifier, &redirect)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_delegation",
    )
    .await;
    // Delegation is recorded in the owner's activity; the owner's own session is untouched.
    let activity: ActivityListResponse = json(
        c.get(format!("{}/api/v1/account/activity", s.url))
            .bearer_auth(token),
    )
    .await;
    assert_eq!(
        activity
            .items
            .iter()
            .filter(|a| a.action == ActivityAction::SessionDelegate)
            .count(),
        4
    );

    // Only the configured client origin may call /api/v1 cross-origin.
    let preflight = |origin: &str, path: &str| {
        c.request(reqwest::Method::OPTIONS, format!("{}{path}", s.url))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "POST")
            .header(
                "Access-Control-Request-Headers",
                "authorization, content-type",
            )
            .send()
    };
    let allowed = preflight(CLIENT_ORIGIN, "/api/v1/auth/delegations/token")
        .await
        .unwrap();
    assert_eq!(
        allowed
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some(CLIENT_ORIGIN)
    );
    assert!(
        allowed
            .headers()
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("authorization")
    );
    assert!(
        allowed
            .headers()
            .get("access-control-allow-credentials")
            .is_none()
    );
    for (origin, path) in [
        ("https://evil.example", "/api/v1/auth/delegations/token"),
        (CLIENT_ORIGIN, "/agent-api/v1/auth/challenges"),
    ] {
        let denied = preflight(origin, path).await.unwrap();
        assert!(
            denied
                .headers()
                .get("access-control-allow-origin")
                .is_none(),
            "{origin} {path}"
        );
    }
    let actual = c
        .get(s.auth("options"))
        .header("Origin", CLIENT_ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(
        actual.headers()["access-control-allow-origin"],
        CLIENT_ORIGIN
    );
    assert!(
        actual.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .contains("x-request-id")
    );
    s.close().await;
}

#[tokio::test]
async fn without_delegation_clients_nothing_is_cross_origin_and_authorization_is_refused() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner: SessionIssuedResponse =
        register(&c, &s, "no-delegation@example.test", "valid password").await;
    error(
        c.post(format!("{}/api/v1/auth/delegations", s.url))
            .bearer_auth(owner.token.expose())
            .json(&CreateDelegationRequest {
                client: "site".into(),
                redirect_uri: "http://localhost:4321/cb".into(),
                code_challenge: challenge_of(&"v".repeat(43)),
                code_challenge_method: CodeChallengeMethod::S256,
            })
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await;
    let response = c
        .get(s.auth("options"))
        .header("Origin", CLIENT_ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    s.close().await;
}
