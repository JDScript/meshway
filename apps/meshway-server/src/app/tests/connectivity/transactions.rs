use super::*;

#[tokio::test]
async fn actual_http_operations_use_node_credentials_and_return_success_for_expected_races() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, _, a, b) = setup(true).await;
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
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
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
