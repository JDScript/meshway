use super::*;
use meshway_protocol::{AgentDataplaneSnapshot, dataplane_digest};

const ACCESS: &str = "/api/v1/access";
const DATAPLANE: &str = "/api/v1/agent/dataplane";

#[tokio::test]
async fn mesh_access_and_dataplane_enforce_distinct_roles_strict_bodies_and_global_cas() {
    let fixture = Fixture::new();
    let enrollment_token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let a = enroll(&fixture, &enrollment_token, &credential, "a")
        .await
        .node;
    let other_credential = local::credential().unwrap();
    let b = enroll(
        &fixture,
        &fixture.token().await.token,
        &other_credential,
        "b",
    )
    .await
    .node;
    let initial = request(&fixture.app, "GET", ACCESS, Some(&fixture.admin), None).await;
    assert_eq!(initial.0, StatusCode::OK);
    assert_eq!(initial.1["policy"], json!({"grants":[]}));
    assert_eq!(initial.1["valid"], true);
    let empty = json!({"expected_revision": initial.1["revision"], "policy":{"grants":[]}});
    for wrong in [
        None,
        Some("invalid"),
        Some(credential.as_str()),
        Some(enrollment_token.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "GET", ACCESS, wrong, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(&fixture.app, "POST", ACCESS, wrong, Some(empty.clone()))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for wrong in [
        None,
        Some("invalid"),
        Some(fixture.admin.as_str()),
        Some(enrollment_token.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "GET", DATAPLANE, wrong, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    // Unbound nodes can fetch an empty effective snapshot, but grants cannot be saved.
    assert_eq!(
        request(&fixture.app, "GET", DATAPLANE, Some(&credential), None)
            .await
            .0,
        StatusCode::OK
    );
    let policy = json!({"grants":[{"id":"forward", "path":[a.id,b.id]}]});
    let unbound = json!({"expected_revision":initial.1["revision"],"policy":policy});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            ACCESS,
            Some(&fixture.admin),
            Some(unbound)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for (token, seed) in [(&credential, 9), (&other_credential, 10)] {
        let bound = request(
            &fixture.app,
            "POST",
            "/api/v1/agent/connection-identity",
            Some(token),
            Some(json!({"public_key":format!("{seed:02x}{}", "00".repeat(31)),"capabilities":[]})),
        )
        .await;
        assert_eq!(bound.0, StatusCode::OK);
    }
    let current = request(&fixture.app, "GET", ACCESS, Some(&fixture.admin), None)
        .await
        .1;
    let save = json!({"expected_revision":current["revision"], "policy":policy});
    let saved = request(
        &fixture.app,
        "POST",
        ACCESS,
        Some(&fixture.admin),
        Some(save.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(
        saved.1["revision"].as_u64().unwrap(),
        current["revision"].as_u64().unwrap() + 1
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            ACCESS,
            Some(&fixture.admin),
            Some(save)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let idempotent = json!({"expected_revision":saved.1["revision"],"policy":policy});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            ACCESS,
            Some(&fixture.admin),
            Some(idempotent.clone())
        )
        .await
        .1,
        saved.1
    );
    let revision = saved.1["revision"].clone();
    for malformed in [
        json!({"expected_revision":revision,"policy":{"grants":[]},"node_id":a.id}),
        json!({"expected_revision":revision,"policy":{"grants":[{"id":"x","path":[a.id,b.id],"reverse":true}]}}),
        json!({"expected_revision":revision,"policy":{}}),
        json!({"expected_revision":"1","policy":{"grants":[]}}),
    ] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                ACCESS,
                Some(&fixture.admin),
                Some(malformed)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for invalid_policy in [
        json!({"grants":[{"id":"x","path":[a.id,a.id]}]}),
        json!({"grants":[{"id":"x","path":[a.id,"missing"]}]}),
        json!({"grants":[{"id":"é","path":[a.id,b.id]}]}),
        json!({"grants":[{"id":"x","path":[a.id,b.id]},{"id":"y","path":[a.id,b.id]}]}),
        json!({"grants":[{"id":"x","path":["a".repeat(128*1024),b.id]}]}),
    ] {
        let body = json!({"expected_revision":revision,"policy":invalid_policy});
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                ACCESS,
                Some(&fixture.admin),
                Some(body)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let too_large = json!({"expected_revision":revision,"policy":{"grants":[{"id":"x","path":["a".repeat(256*1024),b.id]}]}});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            ACCESS,
            Some(&fixture.admin),
            Some(too_large)
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        request(&fixture.app, "GET", ACCESS, Some(&fixture.admin), None)
            .await
            .1,
        saved.1
    );
    let snapshot = request(&fixture.app, "GET", DATAPLANE, Some(&credential), None).await;
    assert_eq!(snapshot.0, StatusCode::OK);
    let typed: AgentDataplaneSnapshot = serde_json::from_value(snapshot.1).unwrap();
    assert_eq!(typed.payload.node_id, a.id);
    assert_eq!(
        typed.payload.access_policy.grants[0].path,
        vec![a.id.clone(), b.id.clone()]
    );
    assert_eq!(typed.digest, dataplane_digest(&typed.payload).unwrap());
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/revoke",
            Some(&fixture.admin),
            Some(json!({"node_id":b.id}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            DATAPLANE,
            Some(&other_credential),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let revoked_source = request(&fixture.app, "GET", ACCESS, Some(&fixture.admin), None)
        .await
        .1;
    assert_eq!(revoked_source["policy"], policy);
    assert_eq!(revoked_source["valid"], false);
    let revoked_effective = request(&fixture.app, "GET", DATAPLANE, Some(&credential), None)
        .await
        .1;
    assert_eq!(revoked_effective["payload"]["access_valid"], false);
    assert_eq!(
        revoked_effective["payload"]["access_policy"],
        json!({"grants":[]})
    );
    assert_eq!(revoked_effective["payload"]["identities"], json!([]));
}

#[test]
fn access_contract_is_in_live_openapi_with_closed_dtos_and_scoped_security() {
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    for (path, verb, operation, role) in [
        (ACCESS, "get", "getMeshAccess", "adminBearer"),
        (ACCESS, "post", "saveMeshAccess", "adminBearer"),
        (DATAPLANE, "get", "getAgentDataplane", "nodeBearer"),
    ] {
        assert_eq!(api["paths"][path][verb]["operationId"], operation);
        assert_eq!(api["paths"][path][verb]["security"], json!([{role:[]}]));
    }
    for schema in [
        "MeshAccessGrant",
        "MeshAccessPolicy",
        "MeshAccessSnapshot",
        "SaveMeshAccessRequest",
        "NodeDataplane",
        "AgentDataplaneSnapshot",
    ] {
        assert_eq!(
            api["components"]["schemas"][schema]["additionalProperties"], false,
            "{schema}"
        );
    }
    assert_eq!(
        api["components"]["schemas"]["MeshAccessGrant"]["properties"]["path"]["minItems"],
        2
    );
    assert_eq!(
        api["components"]["schemas"]["MeshAccessGrant"]["properties"]["path"]["maxItems"],
        8
    );
    assert_eq!(
        api["components"]["schemas"]["NodeDataplane"]["properties"]["identities"]["maxItems"],
        64
    );
}
