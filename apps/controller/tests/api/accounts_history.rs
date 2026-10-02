use super::*;

#[tokio::test]
async fn historical_reads_use_current_network_membership_without_requiring_manage_permission() {
    let f = Fixture::new();
    let base = base(&f).await;
    let owner = f.user_for(base.rsplit('/').next().unwrap()).await;
    let invitation = invite(&f, &base, &owner, "history@test.example", "member").await;
    register(&f, &invitation, "history@test.example").await;
    let member = login(&f, "history@test.example", "member password secret").await;
    let first_token = f.token().await.token;
    let first = enroll(&f, &first_token, &local::credential().unwrap(), "source").await;
    let second_token = f.token().await.token;
    let second = enroll(&f, &second_token, &local::credential().unwrap(), "peer").await;
    let now = chrono::Utc::now();
    let query = json!({
        "source_node_id": first.node.id,
        "peer_node_id": second.node.id,
        "from": (now-chrono::Duration::hours(1)).to_rfc3339(),
        "until": now.to_rfc3339(),
        "limit": 10,
        "after": null,
    });
    let path = format!("{base}/latency/history/query");
    for credential in [&f.admin, &owner, &member] {
        let result = ok(&f, "POST", &path, Some(credential), Some(query.clone())).await;
        assert!(result["records"].as_array().unwrap().is_empty());
    }
    let other = ok(
        &f,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"isolated-history"})),
    )
    .await;
    let other_path = format!(
        "/api/v1/networks/{}/latency/history/query",
        other["network_id"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &other_path,
            Some(&member),
            Some(query.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    ok(
        &f,
        "POST",
        &format!("{base}/members/leave"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        request(&f.app, "POST", &path, Some(&member), Some(query))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // Leaving a network revokes its read permission, not the global browser login.
    ok(&f, "GET", "/api/v1/auth/session", Some(&member), None).await;
}
