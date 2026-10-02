use super::*;

fn policy() -> Value {
    serde_json::to_value(meshway_policy::NetworkPolicy::default_template()).unwrap()
}

#[tokio::test]
async fn current_policy_cas_jsonc_roles_and_inventory_receipts() {
    let f = Fixture::new();
    let token = f.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&f, &token, &credential, "policy-node").await.node;
    let session = f.original_user().await;
    let path = "/api/v1/policy";
    for bearer in [None, Some(credential.as_str()), Some(token.as_str())] {
        assert_eq!(
            request(&f.app, "GET", path, bearer, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/policy/save",
                bearer,
                Some(json!({"expected_revision":2,"policy":policy()}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (status, current) = request(&f.app, "GET", path, Some(&session), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["policy"], policy());
    assert!(current.get("source_mode").is_none());
    assert!(current.get("migration_proposal").is_none());
    let mut edited = policy();
    edited["grants"] = json!([]);
    let text = format!("// preserve draft\n{edited}");
    let save = json!({"expected_revision":current["revision"],"policy":edited,"source_jsonc":text});
    let (status, saved) = request(
        &f.app,
        "POST",
        "/api/v1/policy/save",
        Some(&session),
        Some(save.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["source_jsonc"], text);
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/policy/save",
            Some(&session),
            Some(save)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    for source in ["{}".into(), "x".repeat(256 * 1024 + 1)] {
        assert_eq!(request(&f.app,"POST","/api/v1/policy/save",Some(&session),Some(json!({"expected_revision":saved["revision"],"policy":edited,"source_jsonc":source}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    }
    let noop = request(
        &f.app,
        "POST",
        "/api/v1/policy/save",
        Some(&session),
        Some(json!({"expected_revision":saved["revision"],"policy":edited,"source_jsonc":text})),
    )
    .await;
    assert_eq!(noop.0, StatusCode::OK);
    assert_eq!(noop.1["revision"], saved["revision"]);
    let (_, snapshot) = request(
        &f.app,
        "GET",
        "/api/v1/agents/configuration",
        Some(&credential),
        None,
    )
    .await;
    assert_eq!(snapshot["payload"]["schema_version"], 2);
    assert!(snapshot["payload"].get("routing_policy").is_none());
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(&credential),
            Some(json!({"revision":snapshot["payload"]["revision"],"digest":snapshot["digest"]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(&credential),
            Some(json!({"revision":snapshot["payload"]["revision"],"digest":"0".repeat(64)}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/nodes/country",
            Some(&session),
            Some(json!({"expected_revision":saved["revision"],"node_id":node.id,"country":"us"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/v1/agents/configuration/ack",
            Some(&credential),
            Some(json!({"revision":snapshot["payload"]["revision"],"digest":snapshot["digest"]}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[test]
fn live_contract_has_one_policy_and_current_execution_only() {
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    let paths = api["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v1/networks/{network_id}/policy"));
    assert!(paths.contains_key("/api/v1/networks/{network_id}/policy/save"));
    for path in paths.keys() {
        assert!(!path.contains("policy-v3"));
        assert!(!path.ends_with("/access"));
        assert!(!path.ends_with("/dataplane"));
        assert!(!path.ends_with("/managed-dataplane"));
    }
    let schemas = api["components"]["schemas"].as_object().unwrap();
    for retired in [
        "RoutingPolicy",
        "PolicySourceMode",
        "NodeDataplane",
        "MeshAccessPolicy",
        "SavePolicyRequest",
        "PreviewPolicyRequest",
    ] {
        assert!(!schemas.contains_key(retired), "{retired}");
    }
}
