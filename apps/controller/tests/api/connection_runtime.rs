use super::*;

const BEGIN: &str = "/api/v1/agent/connections/runtime/begin";
const REPORT: &str = "/api/v1/agent/connections/runtime";
const RUNTIMES: &str = "/api/v1/connections/runtime";

async fn publish(fixture: &Fixture, token: &str, seed: u8) {
    let result = request(&fixture.app, "POST", "/api/v1/agent/connection-identity", Some(token), Some(json!({
        "public_key":format!("{seed:02x}{}", "00".repeat(31)),
        "capabilities":[{"transport":"native_tcp","can_dial":true,"can_accept":true,"semantics":"stream"}]
    }))).await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
}

async fn snapshot(fixture: &Fixture, token: &str) -> Value {
    let response = request(
        &fixture.app,
        "GET",
        "/api/v1/agent/connections",
        Some(token),
        None,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    response.1
}

fn report(snapshot: &Value, epoch: u32, sequence: u32) -> Value {
    json!({"epoch":epoch,"sequence":sequence,"revision":snapshot["payload"]["revision"],
        "digest":snapshot["digest"],"state":"applied","reason":"none","listeners":[],"peers":[]})
}

#[tokio::test]
async fn runtime_api_enforces_roles_binding_lease_conflicts_and_saved_only_ack_separation() {
    let fixture = Fixture::new();
    let enrollment_token = fixture.token().await.token;
    let credential = local::credential().unwrap();
    let node = enroll(&fixture, &enrollment_token, &credential, "runtime")
        .await
        .node;
    let other_credential = local::credential().unwrap();
    let other = enroll(
        &fixture,
        &fixture.token().await.token,
        &other_credential,
        "other",
    )
    .await
    .node;
    let begin = json!({"instance_id":"01".repeat(32),"expected_epoch":0});
    let unbound_report = report(&snapshot(&fixture, &credential).await, 1, 1);
    assert_eq!(
        request(&fixture.app, "GET", REPORT, Some(&credential), None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for wrong in [
        None,
        Some("invalid"),
        Some(fixture.admin.as_str()),
        Some(enrollment_token.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "GET", REPORT, wrong, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    for (path, body) in [(BEGIN, begin.clone()), (REPORT, unbound_report.clone())] {
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                path,
                Some(&credential),
                Some(body.clone())
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        for wrong in [
            None,
            Some("invalid"),
            Some(fixture.admin.as_str()),
            Some(enrollment_token.as_str()),
        ] {
            assert_eq!(
                request(&fixture.app, "POST", path, wrong, Some(body.clone()))
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    for wrong in [
        None,
        Some("invalid"),
        Some(credential.as_str()),
        Some(enrollment_token.as_str()),
    ] {
        assert_eq!(
            request(&fixture.app, "GET", RUNTIMES, wrong, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None)
            .await
            .1,
        json!({"nodes":[]})
    );
    publish(&fixture, &credential, 9).await;
    publish(&fixture, &other_credential, 10).await;
    assert_eq!(
        request(&fixture.app, "GET", REPORT, Some(&credential), None).await,
        (StatusCode::OK, json!({"epoch":0}))
    );
    assert_eq!(
        request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None)
            .await
            .1,
        json!({"nodes":[]})
    ); // The epoch read must not create a lease.
    for bad in [
        json!({"instance_id":"01".repeat(32)}),
        json!({"instance_id":"01".repeat(32),"expected_epoch":null}),
        json!({"instance_id":"01".repeat(32),"expected_epoch":-1}),
        json!({"instance_id":"01".repeat(32),"expected_epoch":4294967296_u64}),
    ] {
        assert_eq!(
            request(&fixture.app, "POST", BEGIN, Some(&credential), Some(bad))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    for bad in [
        json!({"instance_id":"AB".repeat(32),"expected_epoch":0}),
        json!({"instance_id":"short","expected_epoch":0}),
    ] {
        assert_eq!(
            request(&fixture.app, "POST", BEGIN, Some(&credential), Some(bad))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(json!({"instance_id":"01".repeat(32),"expected_epoch":0,"node_id":other.id}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            &format!("{BEGIN}?node_id={}", other.id),
            Some(&credential),
            Some(begin.clone())
        )
        .await,
        (StatusCode::OK, json!({"epoch":1}))
    );
    let observation = request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None)
        .await
        .1;
    assert_eq!(observation["nodes"][0]["node_id"], node.id);
    assert_eq!(observation["nodes"][0]["state"], "awaiting_report");
    assert_eq!(observation["nodes"][0]["reported_at"], Value::Null);
    assert_eq!(observation["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            &format!("{REPORT}?node_id={}", other.id),
            Some(&credential),
            None
        )
        .await,
        (StatusCode::OK, json!({"epoch":1}))
    );
    assert_eq!(
        request(&fixture.app, "GET", REPORT, Some(&other_credential), None).await,
        (StatusCode::OK, json!({"epoch":0}))
    );
    let config = snapshot(&fixture, &credential).await;
    let body = report(&config, 1, 1);
    // Another node cannot use this lease/report even though it knows the epoch.
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&other_credential),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let accepted = request(
        &fixture.app,
        "POST",
        REPORT,
        Some(&credential),
        Some(body.clone()),
    )
    .await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(begin.clone())
        )
        .await,
        (StatusCode::OK, json!({"epoch":1}))
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(body.clone())
        )
        .await,
        accepted
    );
    let inventory = request(
        &fixture.app,
        "GET",
        "/api/v1/connections",
        Some(&fixture.admin),
        None,
    )
    .await
    .1;
    for item in inventory["nodes"].as_array().unwrap() {
        assert_eq!(item["acknowledged_revision"], 0);
        assert_eq!(item["acknowledged_at"], Value::Null);
        assert_eq!(item["node"]["acknowledged_revision"], 0);
    }
    assert_eq!(snapshot(&fixture, &credential).await, config);
    let mut changed = body.clone();
    changed["state"] = "withdrawn".into();
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
    let later = report(&config, 1, 3);
    assert_eq!(
        request(&fixture.app, "POST", REPORT, Some(&credential), Some(later))
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
            Some(report(&config, 1, 2))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(json!({"instance_id":"02".repeat(32),"expected_epoch":1}))
        )
        .await
        .1,
        json!({"epoch":2})
    );
    assert_eq!(
        request(&fixture.app, "POST", REPORT, Some(&credential), Some(body))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let body = report(&config, 2, 1);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut wrong_config = report(&config, 2, 2);
    wrong_config["digest"] = "00".repeat(32).into();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(wrong_config)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut wrong_config = report(&config, 2, 2);
    wrong_config["revision"] = (config["payload"]["revision"].as_u64().unwrap() + 1).into();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(wrong_config)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
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
    for (path, body) in [(BEGIN, begin), (REPORT, body)] {
        assert_eq!(
            request(&fixture.app, "POST", path, Some(&credential), Some(body))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(&fixture.app, "GET", REPORT, Some(&credential), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let observation = request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None)
        .await
        .1;
    assert_eq!(observation["nodes"][0]["state"], "withdrawn");
    assert_eq!(observation["nodes"][0]["peers"], json!([]));
    assert_eq!(observation["nodes"][0]["reported_at"], Value::Null);
    assert_eq!(observation["nodes"][0]["current"], false);
}

#[tokio::test]
async fn runtime_begin_cas_rejects_delayed_replays_and_has_one_concurrent_winner() {
    let fixture = Fixture::new();
    let credential = local::credential().unwrap();
    enroll(
        &fixture,
        &fixture.token().await.token,
        &credential,
        "runtime-cas",
    )
    .await;
    publish(&fixture, &credential, 9).await;
    let config = snapshot(&fixture, &credential).await;
    let original_inventory = request(
        &fixture.app,
        "GET",
        "/api/v1/connections",
        Some(&fixture.admin),
        None,
    )
    .await;
    let initial_epoch = request(&fixture.app, "GET", REPORT, Some(&credential), None).await;
    assert_eq!(initial_epoch, (StatusCode::OK, json!({"epoch":0})));
    let a = json!({"instance_id":"01".repeat(32),"expected_epoch":initial_epoch.1["epoch"]});
    let delayed_first =
        json!({"instance_id":"03".repeat(32),"expected_epoch":initial_epoch.1["epoch"]});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(a.clone())
        )
        .await,
        (StatusCode::OK, json!({"epoch":1}))
    );
    let a_report = report(&config, 1, 1);
    let a_receipt = request(
        &fixture.app,
        "POST",
        REPORT,
        Some(&credential),
        Some(a_report.clone()),
    )
    .await;
    assert_eq!(a_receipt.0, StatusCode::OK);
    let a_observation = request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None).await;
    // Original expected=0 survives a lost begin response while A is current.
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(a.clone())
        )
        .await,
        (StatusCode::OK, json!({"epoch":1}))
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(a_report)
        )
        .await,
        a_receipt
    );
    // A first-ever request delayed behind another process cannot steal epoch 1.
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(delayed_first)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None).await,
        a_observation
    );
    let current = request(&fixture.app, "GET", REPORT, Some(&credential), None).await;
    assert_eq!(current.1, json!({"epoch":1}));
    let b = json!({"instance_id":"02".repeat(32),"expected_epoch":current.1["epoch"]});
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(b.clone())
        )
        .await,
        (StatusCode::OK, json!({"epoch":2}))
    );
    let b_report = report(&config, 2, 1);
    let b_receipt = request(
        &fixture.app,
        "POST",
        REPORT,
        Some(&credential),
        Some(b_report.clone()),
    )
    .await;
    assert_eq!(b_receipt.0, StatusCode::OK);
    let b_observation = request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None).await;
    // Concrete replay: A(expected=0)->1, B(expected=1)->2, old A retry must not acquire 3.
    assert_eq!(
        request(&fixture.app, "POST", BEGIN, Some(&credential), Some(a))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&fixture.app, "GET", REPORT, Some(&credential), None).await,
        (StatusCode::OK, json!({"epoch":2}))
    );
    assert_eq!(
        request(&fixture.app, "GET", RUNTIMES, Some(&fixture.admin), None).await,
        b_observation
    );
    assert_eq!(
        request(&fixture.app, "POST", BEGIN, Some(&credential), Some(b)).await,
        (StatusCode::OK, json!({"epoch":2}))
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(b_report)
        )
        .await,
        b_receipt
    );

    let c = json!({"instance_id":"04".repeat(32),"expected_epoch":2});
    let d = json!({"instance_id":"05".repeat(32),"expected_epoch":2});
    let (first, second) = tokio::join!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(c.clone())
        ),
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(d.clone())
        ),
    );
    let loser = match (first.0, second.0) {
        (StatusCode::OK, StatusCode::CONFLICT) => {
            assert_eq!(first.1, json!({"epoch":3}));
            d
        }
        (StatusCode::CONFLICT, StatusCode::OK) => {
            assert_eq!(second.1, json!({"epoch":3}));
            c
        }
        _ => panic!("CAS must have exactly one winner: {first:?}, {second:?}"),
    };
    let winning_report = report(&config, 3, 1);
    let receipt = request(
        &fixture.app,
        "POST",
        REPORT,
        Some(&credential),
        Some(winning_report.clone()),
    )
    .await;
    assert_eq!(receipt.0, StatusCode::OK);
    assert_eq!(
        request(&fixture.app, "POST", BEGIN, Some(&credential), Some(loser))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(winning_report)
        )
        .await,
        receipt
    );
    assert_eq!(snapshot(&fixture, &credential).await, config);
    assert_eq!(
        request(
            &fixture.app,
            "GET",
            "/api/v1/connections",
            Some(&fixture.admin),
            None
        )
        .await,
        original_inventory
    );
}

#[tokio::test]
async fn runtime_api_rejects_out_of_scope_probes_malformed_resources_and_unbounded_bodies() {
    let fixture = Fixture::new();
    let credential = local::credential().unwrap();
    let node = enroll(
        &fixture,
        &fixture.token().await.token,
        &credential,
        "runtime",
    )
    .await
    .node;
    publish(&fixture, &credential, 9).await;
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(json!({"instance_id":"01".repeat(32),"expected_epoch":0}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let config = snapshot(&fixture, &credential).await;
    for (field, value) in [
        ("epoch", json!(0)),
        ("sequence", json!(0)),
        ("state", json!("awaiting_report")),
        ("reason", json!("bind_failed")),
    ] {
        let mut body = report(&config, 1, 1);
        body[field] = value;
        assert_eq!(
            request(&fixture.app, "POST", REPORT, Some(&credential), Some(body))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut body = report(&config, 1, 1);
    body["listeners"] = json!([{"listener_id":"not-configured","state":"bound","reason":"none"}]);
    assert_eq!(
        request(&fixture.app, "POST", REPORT, Some(&credential), Some(body))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let neighbor = json!({"peer_node_id":"unconfigured","transport":"native_tcp","initiator_node_id":node.id,
        "listener_id":"not-configured","candidate_id":null,"state":"authenticated","reason":"none","probe_fresh":true,"probe_rtt_ms":null});
    let mut body = report(&config, 1, 1);
    body["peers"] = json!([neighbor]);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for required in ["listener_id", "candidate_id", "probe_rtt_ms", "reason"] {
        let mut missing = body.clone();
        missing["peers"][0]
            .as_object_mut()
            .unwrap()
            .remove(required);
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                REPORT,
                Some(&credential),
                Some(missing)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (field, value) in [
        ("node_id", json!(node.id)),
        ("reported_at", json!("2099-01-01T00:00:00Z")),
        ("sequence", json!(4294967296_u64)),
    ] {
        let mut invalid = report(&config, 1, 1);
        invalid[field] = value;
        assert_eq!(
            request(
                &fixture.app,
                "POST",
                REPORT,
                Some(&credential),
                Some(invalid)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut raw_error = body.clone();
    raw_error["peers"][0]["reason"] = "DNS lookup of private.example failed".into();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(raw_error)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = body;
    invalid["peers"][0]["endpoint"] = "https://secret.example".into();
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            REPORT,
            Some(&credential),
            Some(invalid)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut huge = report(&config, 1, 1);
    huge["digest"] = "x".repeat(128 * 1024).into();
    assert_eq!(
        request(&fixture.app, "POST", REPORT, Some(&credential), Some(huge))
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            BEGIN,
            Some(&credential),
            Some(json!({"instance_id":"x".repeat(8192),"expected_epoch":0}))
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
