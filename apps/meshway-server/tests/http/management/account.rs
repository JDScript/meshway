use super::*;

#[tokio::test]
async fn account_password_sessions_cli_recovery_and_private_history() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("controller");
    let s = Server::start(&path, true).await;
    let c = client();
    let user = register(&c, &s, "manager@example.test", "valid password").await;
    let other = register(&c, &s, "other@example.test", "valid password").await;
    let second: SessionIssuedResponse = json(c.post(s.auth("login")).json(&LoginRequest {
        email: user.user.email.clone(),
        password: WireSecret::new("valid password".into()),
    }))
    .await;
    let list: AccountSessionsResponse = json(
        c.get(format!("{}/api/v1/account/sessions", s.url))
            .bearer_auth(user.token.expose()),
    )
    .await;
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.items.iter().filter(|s| s.current).count(), 1);
    let first: AccountSessionsResponse = json(
        c.get(format!("{}/api/v1/account/sessions?limit=1", s.url))
            .bearer_auth(user.token.expose()),
    )
    .await;
    assert_eq!(first.items[0].id, second.session.id);
    let cursor = first.next_cursor.unwrap();
    let next_url = query(
        format!("{}/api/v1/account/sessions", s.url),
        &[("cursor", &cursor)],
    );
    reject(
        c.get(next_url.clone()).bearer_auth(other.token.expose()),
        StatusCode::BAD_REQUEST,
        "invalid_cursor",
    )
    .await;
    let later: SessionIssuedResponse = json(c.post(s.auth("login")).json(&LoginRequest {
        email: user.user.email.clone(),
        password: WireSecret::new("valid password".into()),
    }))
    .await;
    let next: AccountSessionsResponse =
        json(c.get(next_url).bearer_auth(user.token.expose())).await;
    assert_eq!(next.as_of, first.as_of);
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].id, user.session.id);
    assert_ne!(next.items[0].id, later.session.id);
    assert!(next.next_cursor.is_none());
    reject(
        c.post(format!(
            "{}/api/v1/account/sessions/{}/revoke",
            s.url, other.session.id
        ))
        .bearer_auth(user.token.expose()),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    empty(
        c.post(format!("{}/api/v1/account/sessions/revoke-others", s.url))
            .bearer_auth(user.token.expose()),
    )
    .await;
    reject(
        c.get(s.auth("session")).bearer_auth(second.token.expose()),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    reject(
        c.post(format!("{}/api/v1/account/password", s.url))
            .bearer_auth(user.token.expose())
            .json(&ChangePasswordRequest {
                current_password: WireSecret::new("wrong password".into()),
                password: WireSecret::new("replacement password".into()),
            }),
        StatusCode::BAD_REQUEST,
        "invalid_current_password",
    )
    .await;
    empty(
        c.post(format!("{}/api/v1/account/password", s.url))
            .bearer_auth(user.token.expose())
            .json(&ChangePasswordRequest {
                current_password: WireSecret::new("valid password".into()),
                password: WireSecret::new("replacement password".into()),
            }),
    )
    .await;
    reject(
        c.get(s.auth("session")).bearer_auth(user.token.expose()),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    s.close().await;
    let issue_reset = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_reset-password"))
            .args([
                "--data-dir",
                path.to_str().unwrap(),
                "--email",
                "manager@example.test",
                "--console-origin",
                "https://controller.example",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let output = String::from_utf8(output.stdout).unwrap();
        output
            .trim()
            .strip_prefix("https://controller.example/reset-password#token=")
            .unwrap()
            .to_owned()
    };
    let replaced_token = issue_reset();
    let token = issue_reset();
    let s = Server::start(&path, false).await;
    for (token, code) in [
        (replaced_token, "password_reset_replaced"),
        ("invalid-reset-token".into(), "invalid_password_reset"),
    ] {
        error(
            c.post(s.auth("password-reset/inspect"))
                .json(&PasswordResetInspectRequest {
                    token: WireSecret::new(token),
                })
                .send()
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            code,
        )
        .await;
    }
    let inspect: PasswordResetInspection = json(c.post(s.auth("password-reset/inspect")).json(
        &PasswordResetInspectRequest {
            token: WireSecret::new(token.clone()),
        },
    ))
    .await;
    assert!(inspect.expires_at > chrono::Utc::now());
    empty(
        c.post(s.auth("password-reset/redeem"))
            .json(&PasswordResetRedeemRequest {
                token: WireSecret::new(token.clone()),
                password: WireSecret::new("after reset password".into()),
            }),
    )
    .await;
    reject(
        c.post(s.auth("password-reset/inspect"))
            .json(&PasswordResetInspectRequest {
                token: WireSecret::new(token),
            }),
        StatusCode::BAD_REQUEST,
        "password_reset_used",
    )
    .await;
    let login: SessionIssuedResponse = json(c.post(s.auth("login")).json(&LoginRequest {
        email: user.user.email,
        password: WireSecret::new("after reset password".into()),
    }))
    .await;
    let response = c
        .get(format!("{}/api/v1/account/activity", s.url))
        .bearer_auth(login.token.expose())
        .send()
        .await
        .unwrap();
    let text = response.text().await.unwrap();
    assert!(!text.contains("mwr_"));
    assert!(!text.contains("passwordHash"));
    let activity: ActivityListResponse = serde_json::from_str(&text).unwrap();
    assert!(
        activity
            .items
            .iter()
            .any(|a| matches!(a.actor, ActivityActor::System {}))
    );
    assert!(activity.items.iter().all(
        |a| !matches!(a.target,ActivityTarget::Account{user_id,..} if user_id==other.user.id)
    ));
    s.close().await;
}
