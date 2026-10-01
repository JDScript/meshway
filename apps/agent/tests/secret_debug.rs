use meshway_agent::generated::types::{
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
