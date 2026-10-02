use super::*;
use meshway_protocol::{DEVICE_CALLBACK_URI, s256};

mod expiry;
mod history;
mod managed;
mod schema;
mod withdrawal;

pub(super) fn is_account_path(path: &str) -> bool {
    path.starts_with("/api/v1/auth/")
        || path.starts_with("/api/v1/me/")
        || [
            "/owner-claim",
            "/members",
            "/invitations",
            "/node-ownership",
        ]
        .iter()
        .any(|s| path.contains(s))
}

async fn ok(
    f: &Fixture,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Value {
    let (status, body) = request(&f.app, method, path, token, body).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    body
}
async fn login(f: &Fixture, email: &str, password: &str) -> String {
    ok(
        f,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email":email,"password":password})),
    )
    .await["session_token"]
        .as_str()
        .unwrap()
        .into()
}
async fn invite(f: &Fixture, base: &str, owner: &str, email: &str, role: &str) -> Value {
    ok(
        f,
        "POST",
        &format!("{base}/invitations"),
        Some(owner),
        Some(json!({"email":email,"role":role,"expires_in_seconds":3600})),
    )
    .await
}
async fn register(f: &Fixture, invitation: &Value, email: &str) -> Value {
    ok(f,"POST","/api/v1/auth/invitations/register",None,Some(json!({"email":email,"password":"member password secret","invitation_token":invitation["invitation_token"]}))).await
}
async fn base(f: &Fixture) -> String {
    let list = ok(f, "GET", "/api/v1/networks", Some(&f.admin), None).await;
    format!(
        "/api/v1/networks/{}",
        list["networks"][0]["network_id"].as_str().unwrap()
    )
}
async fn approval(f: &Fixture, base: &str, session: &str, name: &str) -> String {
    ok(f,"POST",&format!("{base}/device-authorizations"),Some(session),Some(json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_challenge":s256(&"v".repeat(43)),"name":name,"agent_version":"account-test"}))).await["code"].as_str().unwrap().into()
}
fn exchange(credential: &str, name: &str) -> Value {
    json!({"redirect_uri":DEVICE_CALLBACK_URI,"code_verifier":"v".repeat(43),"node_credential":credential,"name":name,"agent_version":"account-test"})
}

#[tokio::test]
async fn accounts_http_multinetwork_rbac_and_public_link_management() {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.user_for(base.rsplit('/').next().unwrap()).await;
    let owner_session = ok(&f, "GET", "/api/v1/auth/session", Some(&owner), None).await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/members/leave"),
            Some(&owner),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/members/save"),
            Some(&owner),
            Some(json!({"user_id":owner_session["user"]["user_id"],"role":"remove"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let second = ok(
        &f,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"other"})),
    )
    .await;
    let other = format!(
        "/api/v1/networks/{}",
        second["network_id"].as_str().unwrap()
    );
    let invitation = invite(&f, &base, &owner, "member@test.example", "member").await;
    let membership = register(&f, &invitation, "member@test.example").await;
    let user = login(&f, " MEMBER@test.example ", "member password secret").await;
    assert_eq!(
        ok(&f, "GET", "/api/v1/networks", Some(&user), None).await["networks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for suffix in [
        "network",
        "nodes",
        "overview",
        "policy",
        "access",
        "tags",
        "connections",
        "connections/runtime",
        "connections/discovery",
        "members",
        "node-ownership",
    ] {
        assert_eq!(
            request(
                &f.app,
                "GET",
                &format!("{base}/{suffix}"),
                Some(&user),
                None
            )
            .await
            .0,
            StatusCode::OK,
            "{suffix}"
        );
        assert_eq!(
            request(
                &f.app,
                "GET",
                &format!("{other}/{suffix}"),
                Some(&owner),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "cross-network {suffix}"
        );
    }
    let network = ok(&f, "GET", &format!("{base}/network"), Some(&owner), None).await;
    let write =
        json!({"expected_revision":network["revision"],"name":"updated","dns":network["dns"]});
    for (suffix, body) in [
        ("network", write.clone()),
        ("tags", json!({"expected_revision":1,"tags":[]})),
        (
            "nodes/tags",
            json!({"expected_revision":1,"node_id":"missing","tags":[]}),
        ),
        (
            "nodes/country",
            json!({"expected_revision":1,"node_id":"missing","country":null}),
        ),
        (
            "access",
            json!({"expected_revision":1,"policy":{"default_action":"allow","rules":[]}}),
        ),
        (
            "policy/save",
            json!({"expected_revision":1,"policy":{"default_action":"reject","rules":[]}}),
        ),
        (
            "connections",
            json!({"expected_revision":1,"configuration":{"listeners":[],"candidates":[]}}),
        ),
        ("enrollment-tokens", json!({"expires_in_seconds":3600})),
        ("nodes/revoke", json!({"node_id":"missing"})),
    ] {
        assert_eq!(
            request(
                &f.app,
                "POST",
                &format!("{base}/{suffix}"),
                Some(&user),
                Some(body)
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "member mutation {suffix}"
        );
    }
    ok(
        &f,
        "POST",
        &format!("{base}/network"),
        Some(&owner),
        Some(write),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/networks",
            Some(&owner),
            Some(json!({"name":"escalation"}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/v1/auth/password-resets",
            Some(&owner),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/members/save"),
            Some(&owner),
            Some(json!({"user_id":membership["user_id"]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let public = &invitation["invitation"]["invitation_id"];
    let detail = ok(
        &f,
        "GET",
        &format!("{base}/invitations/{}", public.as_str().unwrap()),
        Some(&owner),
        None,
    )
    .await;
    assert!(detail["consumed_at"].is_number());
    assert!(
        !detail
            .to_string()
            .contains(invitation["invitation_token"].as_str().unwrap())
    );
    ok(
        &f,
        "GET",
        &format!("{base}/invitations"),
        Some(&owner),
        None,
    )
    .await;
    ok(
        &f,
        "POST",
        &format!("{base}/invitations/revoke"),
        Some(&owner),
        Some(json!({"invitation_id":public})),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "GET",
            &format!("{other}/invitations/{}", public.as_str().unwrap()),
            Some(&f.admin),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Admin can delegate only Member. Losing and regaining delegation does not
    // revive previously issued links, even across a Controller restart.
    ok(
        &f,
        "POST",
        &format!("{base}/members/save"),
        Some(&owner),
        Some(json!({"user_id":membership["user_id"],"role":"admin"})),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/invitations"),
            Some(&user),
            Some(json!({"email":"higher@test.example","role":"owner","expires_in_seconds":3600}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let old = invite(&f, &base, &user, "delegated@test.example", "member").await;
    for role in ["member", "admin"] {
        ok(
            &f,
            "POST",
            &format!("{base}/members/save"),
            Some(&owner),
            Some(json!({"user_id":membership["user_id"],"role":role})),
        )
        .await;
    }
    assert_eq!(request(&f.app,"POST","/api/v1/auth/invitations/register",None,Some(json!({"email":"delegated@test.example","password":"member password secret","invitation_token":old["invitation_token"]}))).await.0,StatusCode::UNAUTHORIZED);
    // Existing user joins a second network with own session and unchanged password.
    let second_invite = invite(&f, &other, &f.admin, "member@test.example", "member").await;
    ok(
        &f,
        "POST",
        "/api/v1/auth/invitations/accept",
        Some(&user),
        Some(json!({"invitation_token":second_invite["invitation_token"]})),
    )
    .await;
    assert_eq!(
        ok(&f, "GET", "/api/v1/networks", Some(&user), None).await["networks"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    ok(
        &f,
        "POST",
        &format!("{other}/members/leave"),
        Some(&user),
        None,
    )
    .await;
    assert_eq!(
        ok(&f, "GET", "/api/v1/networks", Some(&user), None).await["networks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let Fixture {
        directory,
        app,
        admin: _,
    } = f;
    drop(app);
    let app = router(AppState::open(directory.path()).unwrap());
    let (status, metadata) = request(
        &app,
        "GET",
        &format!(
            "{base}/invitations/{}",
            old["invitation"]["invitation_id"].as_str().unwrap()
        ),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(metadata["revoked_at"].is_number());
    assert_eq!(request(&app,"POST","/api/v1/auth/invitations/register",None,Some(json!({"email":"delegated@test.example","password":"member password secret","invitation_token":old["invitation_token"]}))).await.0,StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn accounts_http_app_ownership_revocation_retry_and_atomic_rollback() {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.user_for(base.rsplit('/').next().unwrap()).await;
    let invitation = invite(&f, &base, &owner, "device@test.example", "member").await;
    let membership = register(&f, &invitation, "device@test.example").await;
    let user = login(&f, "device@test.example", "member password secret").await;
    let code = approval(&f, &base, &user, "phone").await;
    let credential = local::credential().unwrap();
    let binding = exchange(&credential, "phone");
    let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
    assert!(
        db.execute("DELETE FROM device_account_bindings", [])
            .is_err()
    );
    db.execute_batch("CREATE TRIGGER inject_ownership BEFORE UPDATE ON node_ownership BEGIN SELECT RAISE(ABORT,'injected secret database failure'); END;").unwrap();
    let (status, error) = request(
        &f.app,
        "POST",
        "/api/v1/devices/exchange",
        Some(&code),
        Some(binding.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!error.to_string().contains("injected"));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM enrollment_tokens WHERE used_node_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER inject_ownership").unwrap();
    let enrolled = ok(
        &f,
        "POST",
        "/api/v1/devices/exchange",
        Some(&code),
        Some(binding.clone()),
    )
    .await;
    assert_eq!(enrolled["node"]["mesh_ipv4"], "100.96.0.1");
    let ownership = ok(
        &f,
        "GET",
        &format!("{base}/node-ownership"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        ownership["nodes"][0]["membership_id"],
        membership["membership_id"]
    );
    // First use depends on browser session; recovery after first use does not.
    let pending = approval(&f, &base, &user, "pending").await;
    ok(&f, "POST", "/api/v1/auth/logout", Some(&user), None).await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&pending),
            Some(exchange(&local::credential().unwrap(), "pending"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ok(
            &f,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(binding.clone())
        )
        .await,
        enrolled
    );
    let revision=ok(&f,"GET",&format!("{base}/overview"),Some(&owner),None).await["configuration_revision"].as_u64().unwrap();
    db.execute_batch("CREATE TRIGGER inject_withdraw BEFORE UPDATE OF revoked_at ON nodes BEGIN SELECT RAISE(ABORT,'withdraw failed'); END;").unwrap();
    let remove = json!({"user_id":membership["user_id"],"role":"remove"});
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/members/save"),
            Some(&owner),
            Some(remove.clone())
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM account_memberships WHERE revoked_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER inject_withdraw").unwrap();
    ok(
        &f,
        "POST",
        &format!("{base}/members/save"),
        Some(&owner),
        Some(remove),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/heartbeat",
            Some(&credential),
            Some(json!({"agent_version":"test"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(ok(&f,"GET",&format!("{base}/overview"),Some(&owner),None).await["configuration_revision"].as_u64().unwrap()>revision);
    let rejoin = invite(&f, &base, &owner, "device@test.example", "member").await;
    let user = login(&f, "device@test.example", "member password secret").await;
    let joined = ok(
        &f,
        "POST",
        "/api/v1/auth/invitations/accept",
        Some(&user),
        Some(json!({"invitation_token":rejoin["invitation_token"]})),
    )
    .await;
    assert_ne!(joined["membership_id"], membership["membership_id"]);
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(binding)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&pending),
            Some(exchange(&local::credential().unwrap(), "pending"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let fresh_code = approval(&f, &base, &user, "fresh").await;
    let fresh_credential = local::credential().unwrap();
    let fresh_request = exchange(&fresh_credential, "fresh");
    ok(
        &f,
        "POST",
        "/api/v1/devices/exchange",
        Some(&fresh_code),
        Some(fresh_request.clone()),
    )
    .await;
    ok(
        &f,
        "POST",
        &format!("{base}/members/leave"),
        Some(&user),
        None,
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/heartbeat",
            Some(&fresh_credential),
            Some(json!({"agent_version":"test"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&fresh_code),
            Some(fresh_request)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn accounts_http_recovery_own_password_session_revocation_and_explicit_claims() {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.user_for(base.rsplit('/').next().unwrap()).await;
    let session = ok(&f, "GET", "/api/v1/auth/session", Some(&owner), None).await;
    let unknown = request(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email":"absent@test.example","password":"incorrect password secret"})),
    )
    .await;
    let incorrect = request(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email":session["user"]["email"],"password":"incorrect password secret"})),
    )
    .await;
    assert_eq!(unknown.0, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown, incorrect);
    let user = session["user"]["user_id"].as_str().unwrap();
    let second = ok(
        &f,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"claim"})),
    )
    .await;
    let other = format!(
        "/api/v1/networks/{}",
        second["network_id"].as_str().unwrap()
    );
    ok(
        &f,
        "POST",
        &format!("{other}/owner-claim"),
        Some(&f.admin),
        Some(json!({"kind":"existing_account","session_token":owner})),
    )
    .await;
    let reset = ok(
        &f,
        "POST",
        "/api/v1/auth/password-resets",
        Some(&f.admin),
        Some(json!({"user_id":user,"expires_in_seconds":3600})),
    )
    .await;
    let id = reset["reset"]["reset_id"].as_str().unwrap();
    let metadata = ok(
        &f,
        "GET",
        &format!("/api/v1/auth/password-resets/{id}"),
        Some(&f.admin),
        None,
    )
    .await;
    assert!(
        !metadata
            .to_string()
            .contains(reset["reset_token"].as_str().unwrap())
    );
    ok(
        &f,
        "GET",
        "/api/v1/auth/password-resets",
        Some(&f.admin),
        None,
    )
    .await;
    ok(
        &f,
        "POST",
        "/api/v1/auth/password-resets/revoke",
        Some(&f.admin),
        Some(json!({"reset_id":id})),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/auth/password-resets/redeem",
            None,
            Some(json!({"reset_token":reset["reset_token"],"new_password":"new password secret"}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let reset = ok(
        &f,
        "POST",
        "/api/v1/auth/password-resets",
        Some(&f.admin),
        Some(json!({"user_id":user,"expires_in_seconds":3600})),
    )
    .await;
    let redeem = json!({"reset_token":reset["reset_token"],"new_password":"new password secret"});
    ok(
        &f,
        "POST",
        "/api/v1/auth/password-resets/redeem",
        None,
        Some(redeem.clone()),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/auth/password-resets/redeem",
            None,
            Some(redeem)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&f.app, "GET", "/api/v1/auth/session", Some(&owner), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let email = session["user"]["email"].as_str().unwrap();
    let new = login(&f, email, "new password secret").await;
    ok(&f,"POST","/api/v1/me/password",Some(&new),Some(json!({"current_password":"new password secret","new_password":"third password secret"}))).await;
    assert_eq!(
        request(&f.app, "GET", "/api/v1/auth/session", Some(&new), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let new = login(&f, email, "third password secret").await;
    ok(
        &f,
        "POST",
        "/api/v1/auth/sessions/revoke",
        Some(&new),
        Some(json!({"user_id":user})),
    )
    .await;
    assert_eq!(
        request(&f.app, "GET", "/api/v1/auth/session", Some(&new), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Ordinary pre-account nodes remain legacy-unassigned until explicit Owner claim.
    let new = login(&f, email, "third password secret").await;
    let token = f.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&f, &token, &credential, "legacy").await.node;
    let membership = &session["memberships"][0]["membership_id"];
    ok(
        &f,
        "POST",
        &format!("{base}/node-ownership/claim"),
        Some(&new),
        Some(json!({"node_id":node.id,"membership_id":membership})),
    )
    .await;
    let ownership = ok(
        &f,
        "GET",
        &format!("{base}/node-ownership"),
        Some(&new),
        None,
    )
    .await;
    assert_eq!(ownership["nodes"][0]["membership_id"], *membership);
}

#[tokio::test]
async fn accounts_http_ingress_budgets_are_bounded_and_ignore_forwarded_source_headers() {
    let f = Fixture::new();
    // Invalid password length avoids KDF; the entry budget still rejects an
    // unlimited stream before lookup/KDF and does not trust forwarding headers.
    for index in 0..11 {
        let status = request(
            &f.app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({"email":"same@test.example","password":"x"})),
        )
        .await
        .0;
        assert_eq!(
            status,
            if index < 10 {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
    for index in 0..20 {
        let req = Request::post("/api/v1/auth/login")
            .header("content-type", "application/json")
            .header("x-forwarded-for", format!("192.0.2.{}", index + 1))
            .body(Body::from(
                json!({"email":format!("{index}@test.example"),"password":"x"}).to_string(),
            ))
            .unwrap();
        let status = f.app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({"email":"another@test.example","password":"x"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn accounts_http_login_and_reset_share_the_same_account_budget() {
    let f = Fixture::new();
    let owner = f.original_user().await;
    let session = ok(&f, "GET", "/api/v1/auth/session", Some(&owner), None).await;
    let reset = ok(
        &f,
        "POST",
        "/api/v1/auth/password-resets",
        Some(&f.admin),
        Some(json!({"user_id":session["user"]["user_id"],"expires_in_seconds":3600})),
    )
    .await;
    // Owner claim + login already reserved two entries for the normalized email.
    for _ in 0..8 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/auth/login",
                None,
                Some(json!({"email":session["user"]["email"],"password":"x"}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(request(&f.app,"POST","/api/v1/auth/password-resets/redeem",None,Some(json!({"reset_token":reset["reset_token"],"new_password":"replacement password secret"}))).await.0,StatusCode::TOO_MANY_REQUESTS);
    // Budget rejection does not consume the reset or replace credentials/session.
    ok(&f, "GET", "/api/v1/auth/session", Some(&owner), None).await;
    let metadata = ok(
        &f,
        "GET",
        &format!(
            "/api/v1/auth/password-resets/{}",
            reset["reset"]["reset_id"].as_str().unwrap()
        ),
        Some(&f.admin),
        None,
    )
    .await;
    assert_eq!(metadata["consumed_at"], Value::Null);
    assert_eq!(metadata["credential_current"], true);
}
