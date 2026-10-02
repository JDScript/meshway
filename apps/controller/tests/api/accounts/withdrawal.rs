use super::*;

async fn withdrawal_cleans_tags(mode: &str) {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.original_user().await;
    let invitation = invite(&f, &base, &owner, "tag-member@fixture.example", "member").await;
    let membership = register(&f, &invitation, "tag-member@fixture.example").await;
    let member = login(&f, "tag-member@fixture.example", "member password secret").await;
    let code = approval(&f, &base, &member, "tagged").await;
    let node = ok(
        &f,
        "POST",
        "/api/v1/devices/exchange",
        Some(&code),
        Some(exchange(&local::credential().unwrap(), "tagged")),
    )
    .await;
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    let tags=ok(&f,"POST",&format!("{base}/tags"),Some(&owner),Some(json!({"expected_revision":tags["revision"],"tags":[{"tag":"tag:orphan","description":"ordinary attribute"}]}))).await;
    ok(&f,"POST",&format!("{base}/nodes/tags"),Some(&owner),Some(json!({"expected_revision":tags["revision"],"node_id":node["node"]["id"],"tags":["tag:orphan"]}))).await;
    let db = rusqlite::Connection::open(f.directory.path().join("controller.sqlite3")).unwrap();
    let before = ok(&f, "GET", &format!("{base}/overview"), Some(&owner), None).await;
    let (path, token, body) = if mode == "leave" {
        (format!("{base}/members/leave"), member.clone(), None)
    } else {
        (
            format!("{base}/members/save"),
            owner.clone(),
            Some(json!({"user_id":membership["user_id"],"role":"remove"})),
        )
    };
    // The failure occurs after node/membership revocation but before tag cleanup.
    db.execute_batch("CREATE TRIGGER fail_tag_cleanup BEFORE DELETE ON node_tags BEGIN SELECT RAISE(ABORT,'fixture cleanup failure'); END;").unwrap();
    assert_eq!(
        request(&f.app, "POST", &path, Some(&token), body.clone())
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        ok(&f, "GET", &format!("{base}/overview"), Some(&owner), None).await,
        before
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
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM nodes WHERE revoked_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM node_tags", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_tag_cleanup").unwrap();
    db.execute_batch("CREATE TRIGGER fail_revision_cleanup BEFORE UPDATE OF revision ON configuration BEGIN SELECT RAISE(ABORT,'fixture revision failure'); END;").unwrap();
    assert_eq!(
        request(&f.app, "POST", &path, Some(&token), body.clone())
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        ok(&f, "GET", &format!("{base}/overview"), Some(&owner), None).await,
        before
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
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM nodes WHERE revoked_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM node_tags", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_revision_cleanup")
        .unwrap();
    ok(&f, "POST", &path, Some(&token), body.clone()).await;
    let revoked_at: String = db
        .query_row(
            "SELECT revoked_at FROM nodes WHERE id=?1",
            [node["node"]["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM node_tags", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Simulate an already-persisted AR2 orphan, then repair via the same API.
    db.execute(
        "INSERT INTO node_tags(network_id,node_id,tag) VALUES(?1,?2,'tag:orphan')",
        rusqlite::params![
            base.rsplit('/').next().unwrap(),
            node["node"]["id"].as_str().unwrap()
        ],
    )
    .unwrap();
    ok(&f, "POST", &path, Some(&token), body.clone()).await;
    assert_eq!(
        db.query_row(
            "SELECT revoked_at FROM nodes WHERE id=?1",
            [node["node"]["id"].as_str().unwrap()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        revoked_at
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM node_tags", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let repaired = ok(&f, "GET", &format!("{base}/overview"), Some(&owner), None).await;
    ok(&f, "POST", &path, Some(&token), body).await;
    assert_eq!(
        ok(&f, "GET", &format!("{base}/overview"), Some(&owner), None).await,
        repaired
    );
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    assert!(tags["assignments"].as_array().unwrap().is_empty());
    // Catalog remains until explicit deletion; no remaining references => delete.
    assert_eq!(tags["tags"].as_array().unwrap().len(), 1);
    ok(
        &f,
        "POST",
        &format!("{base}/tags"),
        Some(&owner),
        Some(json!({"expected_revision":tags["revision"],"tags":[]})),
    )
    .await;
}

#[tokio::test]
async fn remove_cleans_revoked_assignments_atomically_and_idempotently() {
    withdrawal_cleans_tags("remove").await;
}
#[tokio::test]
async fn leave_cleans_revoked_assignments_atomically_and_idempotently() {
    withdrawal_cleans_tags("leave").await;
}

#[tokio::test]
async fn withdrawal_preserves_active_assignments_and_access_references() {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.original_user().await;
    let invitation = invite(
        &f,
        &base,
        &owner,
        "active-tag-member@fixture.example",
        "member",
    )
    .await;
    let membership = register(&f, &invitation, "active-tag-member@fixture.example").await;
    let member = login(
        &f,
        "active-tag-member@fixture.example",
        "member password secret",
    )
    .await;
    let code = approval(&f, &base, &member, "withdraw-tag").await;
    let node = ok(
        &f,
        "POST",
        "/api/v1/devices/exchange",
        Some(&code),
        Some(exchange(&local::credential().unwrap(), "withdraw-tag")),
    )
    .await;
    let other = enroll(
        &f,
        &f.token().await.token,
        &local::credential().unwrap(),
        "unrelated-active",
    )
    .await;
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    ok(&f,"POST",&format!("{base}/tags"),Some(&owner),Some(json!({"expected_revision":tags["revision"],"tags":[{"tag":"tag:shared","description":"retained"}]}))).await;
    for id in [node["node"]["id"].as_str().unwrap(), other.node.id.as_str()] {
        let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
        ok(
            &f,
            "POST",
            &format!("{base}/nodes/tags"),
            Some(&owner),
            Some(json!({"expected_revision":tags["revision"],"node_id":id,"tags":["tag:shared"]})),
        )
        .await;
    }
    ok(
        &f,
        "POST",
        &format!("{base}/members/save"),
        Some(&owner),
        Some(json!({"user_id":membership["user_id"],"role":"remove"})),
    )
    .await;
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    assert_eq!(tags["assignments"].as_array().unwrap().len(), 1);
    assert_eq!(tags["assignments"][0]["node_id"], other.node.id);
    assert_eq!(tags["assignments"][0]["tags"], json!(["tag:shared"]));
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/tags"),
            Some(&owner),
            Some(json!({"expected_revision":tags["revision"],"tags":[]}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Remove the last active assignment, but a policy reference still fences deletion.
    let current = ok(&f, "GET", &format!("{base}/policy"), Some(&owner), None).await;
    let mut policy =
        serde_json::to_value(meshway_policy::NetworkPolicy::default_template()).unwrap();
    policy["grants"] =
        json!([{"id":"tag-reference","src":["tag:shared"],"dst":["*"],"bidirectional":false}]);
    ok(
        &f,
        "POST",
        &format!("{base}/policy/save"),
        Some(&owner),
        Some(json!({"expected_revision":current["revision"],"policy":policy})),
    )
    .await;
    ok(
        &f,
        "POST",
        &format!("{base}/nodes/revoke"),
        Some(&owner),
        Some(json!({"node_id":other.node.id})),
    )
    .await;
    let tags = ok(&f, "GET", &format!("{base}/tags"), Some(&owner), None).await;
    assert!(tags["assignments"].as_array().unwrap().is_empty());
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("{base}/tags"),
            Some(&owner),
            Some(json!({"expected_revision":tags["revision"],"tags":[]}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
