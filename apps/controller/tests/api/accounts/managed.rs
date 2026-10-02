//! Combined frozen P2a + P2b behavior, never touches a live Controller or key.
use super::*;

fn settings(id: &str, node: &str, websocket: bool) -> Value {
    json!({"id":id,"node_id":node,"enabled":true,"transport":"vless","bind_address":"0.0.0.0","bind_port":443,
        "advertised_endpoints":[{"host":"listener.fixture.test","port":443}],
        "vless":{"stream":if websocket {"websocket"} else {"tcp"},"server_name":"meshway.local",
            "websocket_path":if websocket {Some("/meshway")} else {None},
            "websocket_host":if websocket {Some("meshway.local")} else {None}}})
}
async fn key(f: &Fixture, credential: &str, key: &str) {
    ok(f,"POST","/api/v1/agent/connection-identity",Some(credential),Some(json!({"public_key":key.repeat(32),"capabilities":[{"transport":"vless","can_dial":true,"can_accept":true,"semantics":"stream"}]}))).await;
    ok(
        f,
        "POST",
        "/api/v1/agent/managed-transport-capabilities",
        Some(credential),
        Some(json!({"vless_streams":["tcp","websocket"]})),
    )
    .await;
}
async fn node_material(f: &Fixture, credential: &str, path: &str) -> Value {
    let (status, body) = request(&f.app, "GET", path, Some(credential), None).await;
    // Do not Debug/print a secret-bearing success response on assertion failure.
    assert_eq!(status, StatusCode::OK);
    body
}
fn encrypted_rows(db: &rusqlite::Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut q=db.prepare("SELECT network_id,listener_id,node_id,settings,material_version,certificate_sha256,material_nonce,material_ciphertext FROM managed_listeners ORDER BY network_id,listener_id").unwrap();
    let columns = q.column_count();
    q.query_map([], |r| {
        (0..columns)
            .map(|i| r.get(i))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}

#[tokio::test]
async fn managed_permissions_initial_enrollment_legacy_claim_and_membership_withdrawal_are_coherent()
 {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.original_user().await;
    let other = ok(
        &f,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"isolated-materials"})),
    )
    .await;
    let other_base = format!("/api/v1/networks/{}", other["network_id"].as_str().unwrap());
    let other_owner = f.user_for(other["network_id"].as_str().unwrap()).await;
    let invitation = invite(
        &f,
        &base,
        &owner,
        "managed-member@fixture.example",
        "member",
    )
    .await;
    let membership = register(&f, &invitation, "managed-member@fixture.example").await;
    let member = login(
        &f,
        "managed-member@fixture.example",
        "member password secret",
    )
    .await;
    let code = approval(&f, &base, &member, "owned-app").await;
    let credential = local::credential().unwrap();
    let exchange_body = exchange(&credential, "owned-app");
    let node = ok(
        &f,
        "POST",
        "/api/v1/devices/exchange",
        Some(&code),
        Some(exchange_body.clone()),
    )
    .await;
    key(&f, &credential, "11").await;
    let initial =
        json!({"expires_in_seconds":600,"initial_listener":settings("initial-owned","",false)});
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/managed-enrollment-tokens"),
            Some(&member),
            Some(initial.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/managed-enrollment-tokens"),
            Some(&other_owner),
            Some(initial.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            &format!("{base}/managed-listeners"),
            Some(&other_owner),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            &format!("{other_base}/managed-listeners"),
            Some(&member),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let public = ok(
        &f,
        "GET",
        &format!("{base}/managed-listeners"),
        Some(&member),
        None,
    )
    .await;
    let write = json!({"expected_revision":public["revision"],"settings":settings("app-owned",node["node"]["id"].as_str().unwrap(),true)});
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/managed-listeners"),
            Some(&member),
            Some(write.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/managed-listeners"),
            Some(&other_owner),
            Some(write.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    ok(
        &f,
        "POST",
        &format!("{base}/managed-listeners"),
        Some(&owner),
        Some(write),
    )
    .await;
    for token in [&owner, &member, &f.admin] {
        for path in [
            "/api/v1/agent/managed-connections",
            "/api/v1/agent/managed-connections",
        ] {
            assert_eq!(
                request(&f.app, "GET", path, Some(token), None).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/agent/managed-transport-capabilities",
                Some(token),
                Some(json!({"vless_streams":["tcp"]}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let material = node_material(&f, &credential, "/api/v1/agent/managed-connections").await;
    assert_eq!(material["materials"].as_array().unwrap().len(), 1);
    assert!(material["materials"][0]["private_key_pem"].is_string());
    let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
    let key_before =
        local::read_private(&f.directory.path().join("transport-material.key")).unwrap();
    // Existing managed token/initial listener path stays atomic. A failure at
    // material installation must not consume token, reserve IP or create a node.
    let token = ok(
        &f,
        "POST",
        &format!("{base}/managed-enrollment-tokens"),
        Some(&owner),
        Some(initial),
    )
    .await;
    let legacy_credential = local::credential().unwrap();
    let enroll_body = enrollment(&legacy_credential, "legacy-managed");
    db.execute_batch("CREATE TRIGGER managed_account_enroll_abort BEFORE INSERT ON managed_listeners WHEN NEW.listener_id='initial-owned' BEGIN SELECT RAISE(ABORT,'fixture material failure'); END;").unwrap();
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/enroll",
            token["token"].as_str(),
            Some(enroll_body.clone())
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER managed_account_enroll_abort")
        .unwrap();
    let legacy = ok(
        &f,
        "POST",
        "/api/v1/agents/enroll",
        token["token"].as_str(),
        Some(enroll_body.clone()),
    )
    .await;
    assert_eq!(legacy["node"]["mesh_ipv4"], "100.96.0.2");
    key(&f, &legacy_credential, "22").await;
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    let tags=ok(&f,"POST",&format!("{base}/tags"),Some(&owner),Some(json!({"expected_revision":tags["revision"],"tags":[{"tag":"tag:attribute","description":"ordinary node attribute"}]}))).await;
    ok(&f,"POST",&format!("{base}/nodes/tags"),Some(&owner),Some(json!({"expected_revision":tags["revision"],"node_id":legacy["node"]["id"],"tags":["tag:attribute"]}))).await;
    let tag_rows_before: Vec<(String, String, String)> = db
        .prepare("SELECT network_id,node_id,tag FROM node_tags ORDER BY 1,2,3")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let rows_before = encrypted_rows(&db);
    let recovery = ok(
        &f,
        "POST",
        "/api/v1/agents/enroll",
        token["token"].as_str(),
        Some(enroll_body),
    )
    .await;
    assert_eq!(recovery["node"]["id"], legacy["node"]["id"]);
    assert_eq!(encrypted_rows(&db), rows_before);
    let identities_before:Vec<(String,Vec<u8>,String,String,String)>=db.prepare("SELECT n.id,n.credential_hash,n.mesh_ipv4,n.mesh_ipv6,i.public_key FROM nodes n JOIN connection_identities i ON i.node_id=n.id ORDER BY n.id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let ownership = ok(
        &f,
        "GET",
        &format!("{base}/node-ownership"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        ownership["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["node_id"] == legacy["node"]["id"] && n["membership_id"].is_null())
    );
    ok(
        &f,
        "POST",
        &format!("{base}/node-ownership/claim"),
        Some(&owner),
        Some(json!({"node_id":legacy["node"]["id"],"membership_id":membership["membership_id"]})),
    )
    .await;
    assert_eq!(encrypted_rows(&db), rows_before);
    let identities_after:Vec<(String,Vec<u8>,String,String,String)>=db.prepare("SELECT n.id,n.credential_hash,n.mesh_ipv4,n.mesh_ipv6,i.public_key FROM nodes n JOIN connection_identities i ON i.node_id=n.id ORDER BY n.id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(identities_before, identities_after);
    let tag_rows_after: Vec<(String, String, String)> = db
        .prepare("SELECT network_id,node_id,tag FROM node_tags ORDER BY 1,2,3")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(tag_rows_before, tag_rows_after);
    assert!(
        local::read_private(&f.directory.path().join("transport-material.key")).unwrap()
            == key_before
    );
    let after = node_material(&f, &credential, "/api/v1/agent/managed-connections").await;
    assert!(after["materials"] == material["materials"]);
    // An unrelated active node receives only the approved dialer material.
    // After membership withdrawal that projection must be withdrawn as well,
    // retaining the explicit source instead of changing the logical path.
    let observer_token = ok(
        &f,
        "POST",
        &format!("{base}/enrollment-tokens"),
        Some(&owner),
        Some(json!({"expires_in_seconds":600})),
    )
    .await;
    let observer_credential = local::credential().unwrap();
    let observer = ok(
        &f,
        "POST",
        "/api/v1/agents/enroll",
        observer_token["token"].as_str(),
        Some(enrollment(&observer_credential, "observer")),
    )
    .await;
    key(&f, &observer_credential, "33").await;
    let plan = ok(
        &f,
        "GET",
        &format!("{base}/connections"),
        Some(&owner),
        None,
    )
    .await;
    let mut configuration = plan["configuration"].clone();
    configuration["candidates"] = json!([{"id":"observer-to-app","dialer_node_id":observer["node"]["id"],"listener_id":"app-owned","endpoint":{"host":"listener.fixture.test","port":443},"priority":0,"credential_ref":"managed.app-owned"}]);
    ok(
        &f,
        "POST",
        &format!("{base}/connections"),
        Some(&owner),
        Some(json!({"expected_revision":plan["revision"],"configuration":configuration})),
    )
    .await;
    let observer_before = node_material(
        &f,
        &observer_credential,
        "/api/v1/agent/managed-connections",
    )
    .await;
    assert_eq!(observer_before["materials"].as_array().unwrap().len(), 1);
    assert!(observer_before["materials"][0]["private_key_pem"].is_null());
    let before_revision=ok(&f,"GET",&format!("{base}/overview"),Some(&owner),None).await["configuration_revision"].as_u64().unwrap();
    db.execute_batch("CREATE TRIGGER managed_account_withdraw_abort BEFORE UPDATE OF revoked_at ON nodes BEGIN SELECT RAISE(ABORT,'fixture withdrawal failure'); END;").unwrap();
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
            "SELECT COUNT(*) FROM nodes WHERE revoked_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER managed_account_withdraw_abort")
        .unwrap();
    ok(
        &f,
        "POST",
        &format!("{base}/members/save"),
        Some(&owner),
        Some(remove),
    )
    .await;
    assert!(ok(&f,"GET",&format!("{base}/overview"),Some(&owner),None).await["configuration_revision"].as_u64().unwrap()>before_revision);
    for token in [&credential, &legacy_credential] {
        for path in [
            "/api/v1/agent/managed-connections",
            "/api/v1/agent/managed-connections",
        ] {
            assert_eq!(
                request(&f.app, "GET", path, Some(token), None).await.0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/agent/managed-transport-capabilities",
                Some(token),
                Some(json!({"vless_streams":["tcp"]}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/devices/exchange",
            Some(&code),
            Some(exchange_body)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let public = ok(
        &f,
        "GET",
        &format!("{base}/managed-listeners"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        public["listeners"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["compiled_capability_available"] == false)
    );
    assert_eq!(encrypted_rows(&db), rows_before);
    for path in [
        "/api/v1/agent/managed-connections",
        "/api/v1/agent/managed-connections",
    ] {
        let projected = node_material(&f, &observer_credential, path).await;
        assert!(projected["materials"].as_array().unwrap().is_empty());
    }
}
