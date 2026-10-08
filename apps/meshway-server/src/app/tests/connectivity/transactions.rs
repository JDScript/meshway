use super::*;

#[tokio::test]
async fn actual_http_operations_use_node_credentials_and_return_success_for_expected_races() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = crate::http::router(&app);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let url = format!("http://{address}/agent-api/v1/connectivity/udp");
    let mut stale = a.query();
    stale.instance.generation = Version::new(2).unwrap();
    let response = client
        .post(format!("{url}/query"))
        .bearer_auth(&a.token)
        .json(&stale)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<QueryUdpTraversalResponse>().await.unwrap(),
        QueryUdpTraversalResponse::Unavailable {
            reason: UdpTraversalUnavailableReason::ScopeChanged
        }
    );
    let req = request(&app, &a, &b).await;
    let response = client
        .post(format!("{url}/attempts"))
        .bearer_auth(&a.token)
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let PrepareUdpTraversalResponse::Offer { offer } = response.json().await.unwrap() else {
        panic!();
    };
    let response = client
        .post(format!("{url}/attempts/{}/accept", offer.attempt_id))
        .bearer_auth(&a.token)
        .json(&a.cas(&offer))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(matches!(
        response.json::<AcceptUdpTraversalResponse>().await.unwrap(),
        AcceptUdpTraversalResponse::Waiting { .. }
    ));
    let response = client
        .post(format!("{url}/attempts/{}/cancel", offer.attempt_id))
        .bearer_auth(&b.token)
        .json(&b.cas(&offer))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<CancelUdpTraversalResponse>().await.unwrap(),
        CancelUdpTraversalResponse::Cancelled
    );
    let response = client
        .post(format!("{url}/query"))
        .bearer_auth("invalid")
        .json(&a.query())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    app.stream_shutdown()();
    stop.send(()).unwrap();
    server.await.unwrap();
    app.shutdown().await.unwrap();
}

#[test]
fn openapi_registers_the_actual_closed_operations_with_node_security() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    for (suffix, operation) in [
        ("/query", "queryUdpTraversal"),
        ("/attempts", "prepareUdpTraversal"),
        ("/attempts/{attemptId}/accept", "acceptUdpTraversal"),
        ("/attempts/{attemptId}/cancel", "cancelUdpTraversal"),
    ] {
        let route = &spec["paths"][format!("/agent-api/v1/connectivity/udp{suffix}")]["post"];
        assert_eq!(route["operationId"], operation);
        assert_eq!(route["security"], serde_json::json!([{"nodeSession":[]}]));
        assert!(route["responses"].get("409").is_none());
        assert!(route["responses"].get("404").is_none());
    }
}

#[tokio::test]
async fn node_observation_commit_failure_rolls_back_subscription_publication_and_runtime() {
    use meshway_protocol::node_observations::{CreateNodeObservation, NodeObservationTarget};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b, _) = setup(false).await;
    let request = CreateNodeObservation {
        instance: a.instance.clone(),
        subscription_id: meshway_model::ObservationSubscriptionId::new(random_uuid().unwrap())
            .unwrap(),
        target: NodeObservationTarget::MeshIp {
            address: b.target.configuration.addresses.ipv4.unwrap().to_string(),
        },
        service: meshway_configuration::execution::ObservationServiceScope::Tcp {
            port: 443.try_into().unwrap(),
        },
        selection: meshway_protocol::observations::ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    app.executor.handle().execute(|connection| connection.execute_batch("CREATE TEMP TABLE diagnostic_commit_parent(id INTEGER PRIMARY KEY); CREATE TEMP TABLE diagnostic_commit_pending(parent INTEGER REFERENCES diagnostic_commit_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TEMP TRIGGER diagnostic_commit_fail AFTER INSERT ON main.diagnostic_subscriptions WHEN NEW.principal_kind='node_instance' BEGIN INSERT INTO diagnostic_commit_pending VALUES(404); END;").unwrap()).await.unwrap();
    assert!(
        app.observations()
            .create_node_observation(a.token.clone(), request.clone())
            .await
            .is_err()
    );
    app.executor
        .handle()
        .execute(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM diagnostic_subscriptions", [], |r| r
                        .get::<_, i64>(
                        0
                    ))
                    .unwrap(),
                0
            );
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM diagnostic_tasks", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            connection
                .execute_batch("DROP TRIGGER diagnostic_commit_fail;")
                .unwrap();
        })
        .await
        .unwrap();
    assert!(
        app.observations()
            .node_tasks(a.token.clone())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let created = app
        .observations()
        .create_node_observation(a.token.clone(), request.clone())
        .await
        .unwrap();
    let target = sync_execution(&app, &a.token, &a.instance).await;
    assert!(
        target
            .configuration
            .diagnostics
            .contains_key(&created.task.id)
    );
    assert_eq!(
        app.observations()
            .node_tasks(a.token.clone())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    app.executor.handle().execute(|connection| connection.execute_batch("CREATE TEMP TRIGGER diagnostic_commit_fail AFTER UPDATE OF cancelled ON main.diagnostic_subscriptions WHEN NEW.cancelled=1 AND OLD.cancelled=0 BEGIN INSERT INTO diagnostic_commit_pending VALUES(404); END;").unwrap()).await.unwrap();
    assert!(
        app.observations()
            .cancel_node_observation(a.token.clone(), a.instance.clone(), request.subscription_id)
            .await
            .is_err()
    );
    app.executor.handle().execute(|connection| {
        assert_eq!(connection.query_row("SELECT cancelled FROM diagnostic_subscriptions", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        connection.execute_batch("DROP TRIGGER diagnostic_commit_fail; DROP TABLE diagnostic_commit_pending; DROP TABLE diagnostic_commit_parent;").unwrap();
    }).await.unwrap();
    assert_eq!(
        app.observations()
            .node_tasks(a.token.clone())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        app.observations()
            .cancel_node_observation(a.token.clone(), a.instance.clone(), request.subscription_id)
            .await
            .unwrap()
            .task
            .is_none()
    );
    app.shutdown().await.unwrap();
}
