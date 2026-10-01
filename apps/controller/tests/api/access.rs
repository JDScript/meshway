use super::*;
use meshway_protocol::{AgentDataplaneSnapshot, dataplane_digest};

const ACCESS: &str = "/api/v1/access";
const DATAPLANE: &str = "/api/v1/agent/dataplane";

#[tokio::test]
async fn endpoint_access_tags_preview_and_dataplane_enforce_roles_and_saved_contracts() {
    let fixture = Fixture::new();
    let credential_a = local::credential().unwrap();
    let credential_b = local::credential().unwrap();
    let a = enroll(&fixture, &fixture.token().await.token, &credential_a, "a")
        .await
        .node;
    let b = enroll(&fixture, &fixture.token().await.token, &credential_b, "b")
        .await
        .node;
    let initial = request(&fixture.app, "GET", ACCESS, Some(&fixture.admin), None).await;
    assert_eq!(initial.0, StatusCode::OK);
    assert_eq!(
        initial.1["policy"],
        json!({"default_action":"reject","rules":[]})
    );
    assert_eq!(initial.1["source_jsonc"], Value::Null);
    for token in [None, Some(credential_a.as_str())] {
        assert_eq!(
            request(&fixture.app, "GET", ACCESS, token, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(&fixture.app, "GET", "/api/v1/tags", token, None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let tags = request(
        &fixture.app,
        "POST",
        "/api/v1/tags",
        Some(&fixture.admin),
        Some(json!({
            "expected_revision":initial.1["revision"],
            "tags":[{"tag":"tag:prod","description":"production"}]
        })),
    )
    .await;
    assert_eq!(tags.0, StatusCode::OK, "{}", tags.1);
    let assigned = request(
        &fixture.app,
        "POST",
        "/api/v1/nodes/tags",
        Some(&fixture.admin),
        Some(json!({
            "expected_revision":tags.1["revision"],"node_id":a.id,"tags":["tag:prod"]
        })),
    )
    .await;
    assert_eq!(assigned.0, StatusCode::OK, "{}", assigned.1);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/tags",
            Some(&credential_a),
            Some(json!({
                "expected_revision":assigned.1["revision"],"node_id":a.id,"tags":[]
            }))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/tags",
            Some(&fixture.admin),
            Some(json!({
                "expected_revision":assigned.1["revision"],"node_id":a.id,"tags":[],"self_tag":true
            }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let policy = json!({"default_action":"reject","rules":[{
        "id":"prod-to-b","source":["tag:prod"],"destination":[format!("node:{}",b.id)],
        "action":"allow","bidirectional":true
    }]});
    let source = format!(
        "{{ // retained\n \"default_action\":\"reject\", \"rules\":{} ,}}",
        policy["rules"]
    );
    let saved = request(
        &fixture.app,
        "POST",
        ACCESS,
        Some(&fixture.admin),
        Some(json!({
            "expected_revision":assigned.1["revision"],"policy":policy,"source_jsonc":source
        })),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(saved.1["source_jsonc"], source);
    assert_eq!(request(&fixture.app, "POST", ACCESS, Some(&fixture.admin), Some(json!({
        "expected_revision":saved.1["revision"],"policy":policy,
        "source_jsonc":"{\"default_action\":\"reject\",\"default_action\":\"reject\",\"rules\":[]}"
    }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    let routing = request(
        &fixture.app,
        "POST",
        "/api/v1/policy/save",
        Some(&fixture.admin),
        Some(json!({
            "expected_revision":saved.1["revision"],"policy":{"default_action":"direct","rules":[]}
        })),
    )
    .await;
    assert_eq!(routing.0, StatusCode::OK, "{}", routing.1);
    for (credential, seed) in [(&credential_a, 9), (&credential_b, 10)] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                "/api/v1/agent/connection-identity",
                Some(credential),
                Some(json!({
                    "public_key":format!("{seed:02x}{}", "00".repeat(31)),"capabilities":[]
                }))
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let preview = request(
        &fixture.app,
        "POST",
        "/api/v1/flows/preview",
        Some(&fixture.admin),
        Some(json!({"flow":{
            "source_node_id":a.id,"destination_node_id":b.id,"destination_ip":b.mesh_ipv4,
            "domain":null,"destination_country":null
        }})),
    )
    .await;
    assert_eq!(preview.0, StatusCode::OK, "{}", preview.1);
    assert_eq!(preview.1["authorization"]["action"], "allow");
    assert_eq!(preview.1["authorization"]["matched_direction"], "forward");
    assert_eq!(preview.1["routing"]["action"], "route");
    assert_eq!(preview.1["execution_supported"], true);
    let contextual = request(
        &fixture.app,
        "POST",
        "/api/v1/flows/preview",
        Some(&fixture.admin),
        Some(json!({"flow":{
            "source_node_id":a.id,"destination_node_id":b.id,"destination_ip":b.mesh_ipv4,
            "domain":"example.com","destination_country":null
        }})),
    )
    .await;
    assert_eq!(contextual.0, StatusCode::OK);
    assert_eq!(contextual.1["execution_supported"], false);

    let snapshot = request(&fixture.app, "GET", DATAPLANE, Some(&credential_a), None).await;
    let typed: AgentDataplaneSnapshot = serde_json::from_value(snapshot.1).unwrap();
    assert_eq!(typed.payload.access_policy.grants.len(), 2);
    assert!(
        typed
            .payload
            .access_policy
            .grants
            .iter()
            .any(|grant| grant.path == [a.id.clone(), b.id.clone()])
    );
    assert_eq!(typed.digest, dataplane_digest(&typed.payload).unwrap());
}

#[test]
fn access_v2_contract_is_live_closed_annotated_and_legacy_dataplane_only_contains_grants() {
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    for (path, verb, operation, role) in [
        (ACCESS, "get", "getMeshAccess", "adminBearer"),
        (ACCESS, "post", "saveMeshAccess", "adminBearer"),
        ("/api/v1/tags", "get", "getTags", "adminBearer"),
        ("/api/v1/tags", "post", "saveTags", "adminBearer"),
        ("/api/v1/nodes/tags", "post", "setNodeTags", "adminBearer"),
        (
            "/api/v1/flows/preview",
            "post",
            "previewFlow",
            "adminBearer",
        ),
        (DATAPLANE, "get", "getAgentDataplane", "nodeBearer"),
    ] {
        assert_eq!(
            api["paths"][contract_path(path)][verb]["operationId"],
            operation
        );
        assert_eq!(
            api["paths"][contract_path(path)][verb]["security"],
            json!([{role:[]}])
        );
    }
    for schema in [
        "MeshAccessPolicy",
        "MeshAccessRule",
        "MeshPathAuthorization",
        "TagSnapshot",
        "PreviewFlowResponse",
    ] {
        assert_eq!(
            api["components"]["schemas"][schema]["additionalProperties"], false,
            "{schema}"
        );
    }
    assert_eq!(
        api["components"]["schemas"]["NodeDataplane"]["properties"]["access_policy"]["$ref"],
        "#/components/schemas/MeshPathAuthorization"
    );
    assert_eq!(
        api["components"]["schemas"]["MeshAccessRule"]["properties"]["source"]["items"]["x-meshway-ref"],
        "selector"
    );
    assert_eq!(
        api["components"]["schemas"]["FlowQuery"]["properties"]["source_node_id"]["x-meshway-ref"],
        "node"
    );
    assert_eq!(
        api["components"]["schemas"]["TagDefinition"]["properties"]["tag"]["x-meshway-ref"],
        "tag"
    );
}
