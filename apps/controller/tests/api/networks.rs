use super::*;
use meshway_protocol::{NetworkList, NetworkSnapshot};

#[tokio::test]
async fn network_resources_and_credentials_enforce_isolation_under_interleaved_requests() {
    let f = Fixture::new();
    for token in [None, Some("00".repeat(32)).as_deref()] {
        assert_eq!(
            request(&f.app, "GET", "/api/v1/networks", token, None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/v1/networks",
                token,
                Some(json!({"name":"Other"}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (_, body) = request(&f.app, "GET", "/api/v1/networks", Some(&f.admin), None).await;
    let initial: NetworkList = serde_json::from_value(body).unwrap();
    let first = &initial.networks[0];
    let (_, body) = request(
        &f.app,
        "POST",
        "/api/v1/networks",
        Some(&f.admin),
        Some(json!({"name":"Other"})),
    )
    .await;
    let second: NetworkSnapshot = serde_json::from_value(body).unwrap();
    assert_eq!(second.revision, 1);
    let first_path = format!("/api/v1/networks/{}", first.network_id);
    let second_path = format!("/api/v1/networks/{}", second.network_id);
    let mut enrolled = Vec::new();
    for base in [&first_path, &second_path] {
        let (_, token) = request(
            &f.app,
            "POST",
            &format!("{base}/enrollment-tokens"),
            Some(&f.admin),
            Some(json!({"expires_in_seconds":600})),
        )
        .await;
        let token: CreateEnrollmentTokenResponse = serde_json::from_value(token).unwrap();
        let credential = local::credential().unwrap();
        let node = enroll(&f, &token.token, &credential, "same name")
            .await
            .node;
        enrolled.push((node, credential));
    }
    assert_eq!(enrolled[0].0.mesh_ipv4, enrolled[1].0.mesh_ipv4);
    for (operation, body) in [
        ("nodes/revoke", json!({"node_id":enrolled[0].0.id})),
        (
            "nodes/country",
            json!({"node_id":enrolled[0].0.id,"country":"US","expected_revision":2}),
        ),
        (
            "nodes/tags",
            json!({"node_id":enrolled[0].0.id,"tags":[],"expected_revision":2}),
        ),
        (
            "access",
            json!({"expected_revision":2,"policy":{"default_action":"reject","rules":[{"id":"cross","source":[format!("node:{}",enrolled[0].0.id)],"destination":["*"],"action":"allow","bidirectional":false}]}}),
        ),
        (
            "policy/save",
            json!({"expected_revision":2,"policy":{"default_action":"direct","rules":[{"id":"cross","source":{"node_ids":[enrolled[0].0.id],"countries":[]},"destination":{"ip_cidrs":[],"domain_suffixes":[],"countries":[]},"action":{"kind":"route","via":[],"exit_node_id":null}}]}}),
        ),
        (
            "connections",
            json!({"expected_revision":2,"configuration":{"listeners":[{"id":"cross","node_id":enrolled[0].0.id,"transport":"native_udp","bind":"0.0.0.0:42000","advertised_endpoints":[],"credential_ref":null}],"candidates":[]}}),
        ),
    ] {
        let status = request(
            &f.app,
            "POST",
            &format!("{second_path}/{operation}"),
            Some(&f.admin),
            Some(body),
        )
        .await
        .0;
        assert!(
            status.is_client_error(),
            "cross-network {operation} returned {status}"
        );
    }
    // Mutations with identical CAS versions in separate networks both succeed.
    let first_tags = format!("{first_path}/tags");
    let second_tags = format!("{second_path}/tags");
    let (a, b) = tokio::join!(
        request(
            &f.app,
            "POST",
            &first_tags,
            Some(&f.admin),
            Some(
                json!({"expected_revision":2,"tags":[{"tag":"tag:shared","description":"first"}]})
            )
        ),
        request(
            &f.app,
            "POST",
            &second_tags,
            Some(&f.admin),
            Some(
                json!({"expected_revision":2,"tags":[{"tag":"tag:shared","description":"second"}]})
            )
        )
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    for i in 0..16 {
        let index = i % 2;
        let base = [&first_path, &second_path][index];
        let (_, body) = request(
            &f.app,
            "GET",
            &format!("{base}/nodes"),
            Some(&f.admin),
            None,
        )
        .await;
        let nodes: NodeList = serde_json::from_value(body).unwrap();
        assert_eq!(nodes.nodes.len(), 1);
        assert_eq!(nodes.nodes[0].id, enrolled[index].0.id);
        let (_, config) = request(
            &f.app,
            "GET",
            "/api/v1/agents/configuration?network_id=wrong",
            Some(&enrolled[index].1),
            None,
        )
        .await;
        assert_eq!(config["payload"]["node"]["id"], enrolled[index].0.id);
        assert_eq!(config["payload"]["peers"], json!([]));
        let (_, tags) = request(&f.app, "GET", &format!("{base}/tags"), Some(&f.admin), None).await;
        assert_eq!(
            tags["tags"][0]["description"],
            if index == 0 { "first" } else { "second" }
        );
    }
    let (_, code) = request(&f.app,"POST",&format!("{second_path}/device-authorizations"),Some(&f.admin),Some(json!({"redirect_uri":meshway_protocol::DEVICE_CALLBACK_URI,"code_challenge":meshway_protocol::s256(&"v".repeat(43)),"name":"App","agent_version":"test"}))).await;
    let (_, exchange) = request(&f.app,"POST","/api/v1/devices/exchange",code["code"].as_str(),Some(json!({"redirect_uri":meshway_protocol::DEVICE_CALLBACK_URI,"code_verifier":"v".repeat(43),"node_credential":local::credential().unwrap(),"name":"App","agent_version":"test"}))).await;
    assert_eq!(exchange["node"]["mesh_ipv4"], "100.96.0.2");
    let (_, first_nodes) = request(
        &f.app,
        "GET",
        &format!("{first_path}/nodes"),
        Some(&f.admin),
        None,
    )
    .await;
    assert_eq!(first_nodes["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(
        request(&f.app, "GET", "/api/v1/nodes", Some(&f.admin), None)
            .await
            .0,
        StatusCode::OK
    ); // fixture shorthand expands to a scoped path
    let legacy = f
        .app
        .clone()
        .oneshot(
            Request::get("/api/v1/nodes")
                .header("authorization", format!("Bearer {}", f.admin))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::NOT_FOUND);
}

#[test]
fn all_network_admin_operations_declare_required_scope() {
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    for (path, item) in api["paths"].as_object().unwrap() {
        for operation in item.as_object().unwrap().values() {
            if operation["security"] == json!([{"adminBearer":[]}]) && path != "/api/v1/networks" {
                assert!(path.starts_with("/api/v1/networks/{network_id}/"));
                assert_eq!(operation["parameters"][0]["name"], "network_id");
                assert_eq!(operation["parameters"][0]["required"], true);
            }
        }
    }
}
