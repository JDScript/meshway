use super::*;

const REPORT: &str = "/api/v1/agent/connections/discovery";
const SNAPSHOT: &str = "/api/v1/connections/discovery";

#[tokio::test]
async fn discovery_routes_auth_json_contract_lease_and_openapi() {
    let fixture = Fixture::new();
    let enrollment = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &enrollment, &credential, "discovery")
        .await
        .node;
    let (_, initial) = request(&fixture.app, "GET", SNAPSHOT, Some(&fixture.admin), None).await;
    assert_eq!(initial["nodes"][0]["node_id"], node.id);
    assert_eq!(initial["nodes"][0]["state"], "awaiting_report");
    assert_eq!(initial["nodes"][0]["reported_at"], Value::Null);
    let report = json!({"epoch":1,"sequence":1,"revision":1,"digest":"00".repeat(32),"state":"disabled","sockets":[]});
    for token in [
        None,
        Some("invalid"),
        Some(fixture.admin.as_str()),
        Some(enrollment.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "POST", REPORT, token, Some(report.clone()))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for token in [
        None,
        Some("invalid"),
        Some(credential.as_str()),
        Some(enrollment.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "GET", SNAPSHOT, token, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(report.clone())
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(request(&fixture.app, "POST", "/api/v1/agent/connection-identity", Some(&credential), Some(json!({"public_key":format!("09{}", "00".repeat(31)), "capabilities":[{"transport":"native_udp","can_dial":true,"can_accept":true,"semantics":"datagram"}]}))).await.0, StatusCode::OK);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agent/connections/runtime/begin",
            Some(&credential),
            Some(json!({"instance_id":"01".repeat(32),"expected_epoch":0}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, configuration) = request(
        &fixture.app,
        "GET",
        "/api/v1/agent/connections",
        Some(&credential),
        None,
    )
    .await;
    let report = json!({"epoch":1,"sequence":1,"revision":configuration["payload"]["revision"],"digest":configuration["digest"],"state":"disabled","sockets":[]});
    let (status, first) = request(
        &fixture.app,
        "POST",
        REPORT,
        Some(&credential),
        Some(report.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(report.clone())
        )
        .await
        .1,
        first
    );
    let (_, startup) = request(&fixture.app, "GET", SNAPSHOT, Some(&fixture.admin), None).await;
    assert_eq!(startup["nodes"][0]["current"], false); // no Applied evidence yet
    let runtime = json!({"epoch":1,"sequence":1,"revision":configuration["payload"]["revision"],"digest":configuration["digest"],"state":"applied","reason":"none","listeners":[],"peers":[]});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/agent/connections/runtime",
            Some(&credential),
            Some(runtime)
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, disabled) = request(&fixture.app, "GET", SNAPSHOT, Some(&fixture.admin), None).await;
    assert_eq!(disabled["nodes"][0]["state"], "disabled");
    assert_eq!(disabled["nodes"][0]["current"], true);
    assert_eq!(disabled["network_id"], initial["network_id"]);
    assert!(!disabled.to_string().contains(&credential));
    assert!(!disabled.to_string().contains(&fixture.admin));
    let mut changed = report.clone();
    changed["state"] = json!("withdrawn");
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(changed)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let now = chrono::Utc::now();
    let socket = json!({"resource_kind":"listener","resource_id":"foreign","generation":1,"socket_id":1,"bound_address":"0.0.0.0:123","source":"managed_udp_socket_stun","observations":[{"observer":"192.0.2.1:3478","outcome":"timeout","mapped_endpoint":null,"rtt_ms":null,"observed_at":now,"expires_at":now+chrono::Duration::seconds(20)}]});
    for field in ["mapped_endpoint", "rtt_ms"] {
        let mut malformed = report.clone();
        malformed["sequence"] = json!(2);
        malformed["state"] = json!("probing");
        malformed["sockets"] = json!([socket.clone()]);
        malformed["sockets"][0]["observations"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                REPORT,
                Some(&credential),
                Some(malformed)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for pointer in ["", "/sockets/0", "/sockets/0/observations/0"] {
        let mut malformed = report.clone();
        malformed["sockets"] = json!([socket.clone()]);
        malformed
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(true));
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                REPORT,
                Some(&credential),
                Some(malformed)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/nodes/revoke",
            Some(&fixture.admin),
            Some(json!({"node_id":node.id}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(report)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, revoked) =
        request(&fixture.app, "GET", SNAPSHOT, Some(&fixture.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["nodes"][0]["state"], "withdrawn");
    assert_eq!(revoked["nodes"][0]["sockets"], json!([]));
    let api: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    assert_eq!(
        api["paths"][REPORT]["post"]["operationId"],
        "reportEndpointDiscovery"
    );
    assert_eq!(
        api["paths"][REPORT]["post"]["security"],
        json!([{"nodeBearer":[]}])
    );
    assert_eq!(
        api["paths"][contract_path(SNAPSHOT)]["get"]["operationId"],
        "getEndpointDiscoveries"
    );
    assert_eq!(
        api["paths"][contract_path(SNAPSHOT)]["get"]["security"],
        json!([{"managementBearer":[]}])
    );
    let required = api["components"]["schemas"]["StunEndpointObservation"]["required"]
        .as_array()
        .unwrap();
    assert!(required.contains(&json!("mapped_endpoint")) && required.contains(&json!("rtt_ms")));
}
