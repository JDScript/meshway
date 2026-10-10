use super::*;

#[test]
fn configuration_schema_exposes_optional_strict_transport_protocol_array() {
    let response = crate::configuration::catalog::schema(rid()).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&response.schema_json).unwrap();
    let rule = &schema["$defs"]["TopologyRule"];
    assert_eq!(rule["properties"]["transports"]["type"], "array");
    assert_eq!(
        rule["properties"]["transports"]["items"]["$ref"],
        "#/$defs/TransportProtocol"
    );
    assert!(
        !rule["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("transports"))
    );
    assert_eq!(
        schema["$defs"]["TransportProtocol"]["enum"],
        serde_json::json!(["native_udp", "native_tcp", "vless", "trojan"])
    );
}
