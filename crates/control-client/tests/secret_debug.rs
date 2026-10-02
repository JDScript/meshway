use meshway_control_client::types::{
    AgentManagedConnectionsSnapshot, EnrollAgentRequest, ExchangeDeviceRequest,
    ManagedVlessMaterial,
};

#[test]
fn generated_secret_debug_redacts_direct_and_nested_sentinels_without_changing_wire() {
    let sentinel = "SECRET_SENTINEL_DO_NOT_LOG";
    let enrollment: EnrollAgentRequest = serde_json::from_value(
        serde_json::json!({"node_credential":sentinel,"name":"test","agent_version":"test"}),
    )
    .unwrap();
    let exchange:ExchangeDeviceRequest=serde_json::from_value(serde_json::json!({"node_credential":sentinel,"name":"test","agent_version":"test","code_verifier":sentinel,"redirect_uri":"meshway://enroll/callback"})).unwrap();
    let material:ManagedVlessMaterial=serde_json::from_value(serde_json::json!({"listener_id":"x","credential_ref":"managed.x","version":1,"settings":{"stream":"websocket","server_name":"meshway.test","websocket_path":"/meshway","websocket_host":"meshway.test"},"uuid":sentinel,"certificate_pem":"public","certificate_sha256":"public","private_key_pem":sentinel})).unwrap();
    for debug in [
        format!("{enrollment:?}"),
        format!("{exchange:?}"),
        format!("{material:?}"),
    ] {
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains(sentinel));
    }
    assert!(
        serde_json::to_string(&enrollment)
            .unwrap()
            .contains(sentinel)
    );
    let value = serde_json::json!({"network_id":"0".repeat(32),"connections":{"digest":"0".repeat(64),"payload":{"schema_version":1,"revision":1,"node_id":"a".repeat(64),"public_key":null,"capabilities":[],"valid":true,"diagnostics":[],"listeners":[],"outgoing":[],"incoming":[],"peers":[]}},"materials":[serde_json::to_value(&material).unwrap()],"digest":"0".repeat(64)});
    let nested: AgentManagedConnectionsSnapshot = serde_json::from_value(value).unwrap();
    assert!(!format!("{nested:?}").contains(sentinel));
    assert!(serde_json::to_string(&nested).unwrap().contains(sentinel));
}

#[tokio::test]
async fn real_private_success_errors_discard_secrets_in_display_debug_and_envelopes() {
    use axum::{Router, http::StatusCode, routing::get};
    const UUID_SECRET: &str = "UUID_HTTP_SECRET_SENTINEL";
    const KEY_SECRET: &str = "PRIVATE_KEY_HTTP_SECRET_SENTINEL";
    let base = serde_json::json!({"network_id":"0".repeat(32),"connections":{"digest":"0".repeat(64),"payload":{"schema_version":1,"revision":1,"node_id":"a".repeat(64),"public_key":null,"capabilities":[],"valid":true,"diagnostics":[],"listeners":[],"outgoing":[],"incoming":[],"peers":[]}},"materials":[{"listener_id":"x","credential_ref":"managed.x","version":1,"settings":{"stream":"tcp","server_name":"meshway.test","websocket_path":null,"websocket_host":null},"uuid":UUID_SECRET,"certificate_pem":"public","certificate_sha256":"public","private_key_pem":KEY_SECRET}],"digest":"0".repeat(64)});
    let mut unknown = base.clone();
    unknown["unexpected"] = UUID_SECRET.into();
    let mut bad_enum = base.clone();
    bad_enum["materials"][0]["settings"]["stream"] = KEY_SECRET.into();
    for (status, body, category) in [
        (
            StatusCode::OK,
            format!("{{\"uuid\":\"{UUID_SECRET}\",\"private_key_pem\":\"{KEY_SECRET}\", not-json"),
            "success_response_decode",
        ),
        (
            StatusCode::OK,
            unknown.to_string(),
            "success_response_decode",
        ),
        (
            StatusCode::OK,
            bad_enum.to_string(),
            "success_response_decode",
        ),
        (
            StatusCode::CREATED,
            base.to_string(),
            "unexpected_success_status",
        ),
    ] {
        let app = Router::new().route(
            "/api/v1/agent/managed-connections",
            get(move || {
                let body = body.clone();
                async move { (status, body) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let error = meshway_control_client::HttpClient::new()
            .with_base_url(origin)
            .with_api_key("a".repeat(64))
            .get_agent_managed_connections()
            .await
            .unwrap_err();
        let envelope = error.api().unwrap();
        assert_eq!(envelope.status, status.as_u16());
        assert_eq!(envelope.category(), category);
        assert!(!envelope.is_retryable());
        assert!(envelope.body.is_empty());
        assert!(envelope.raw_body.is_empty());
        for text in [
            format!("{error}"),
            format!("{error:?}"),
            format!("{envelope}"),
            format!("{envelope:?}"),
            format!("{:?}", Some(&error)),
        ] {
            assert!(!text.contains(UUID_SECRET));
            assert!(!text.contains(KEY_SECRET));
        }
        assert!(
            envelope
                .parse_error
                .as_ref()
                .is_none_or(|text| !text.contains(UUID_SECRET) && !text.contains(KEY_SECRET))
        );
        server.abort();
        let _ = server.await;
    }
}
