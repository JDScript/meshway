use meshway_protocol::{InstanceLease, NodeChallenge, QueryUdpTraversalResponse};
use serde_json::json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(components(schemas(InstanceLease, NodeChallenge, QueryUdpTraversalResponse)))]
struct Api;

#[test]
fn timestamp_fields_reference_the_same_named_format_and_nullable_lease_stays_required() {
    let document = serde_json::to_value(Api::openapi()).unwrap();
    let schemas = &document["components"]["schemas"];
    assert_eq!(schemas["Timestamp"]["type"], "string");
    assert_eq!(schemas["Timestamp"]["format"], "meshway-timestamp-ms");
    // A field may carry its own description next to the reference; the
    // referenced format stays the single definition of the value.
    let timestamp = json!("#/components/schemas/Timestamp");
    for name in ["InstanceLease", "NodeChallenge"] {
        let field = &schemas[name]["properties"]["expiresAt"];
        assert_eq!(field["$ref"], timestamp);
        assert!(field.get("type").is_none() && field.get("format").is_none());
    }
    let optional = &schemas["InstanceLease"]["properties"]["executionExpiresAt"];
    let variants = optional["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0]["$ref"], timestamp);
    assert_eq!(variants[1], json!({"type":"null"}));
    assert!(
        schemas["InstanceLease"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("executionExpiresAt"))
    );
    let current = schemas["QueryUdpTraversalResponse"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"]["status"]["enum"] == json!(["current"]))
        .unwrap();
    for name in ["asOf", "expiresAt"] {
        assert_eq!(current["properties"][name]["$ref"], timestamp);
    }
    let lease = json!({"instanceId":"00000000-0000-4000-8000-000000000001","generation":1,"expiresAt":"2026-10-04T00:00:00Z","executionExpiresAt":null,"target":null});
    assert!(serde_json::from_value::<InstanceLease>(lease.clone()).is_ok());
    let mut missing = lease;
    missing
        .as_object_mut()
        .unwrap()
        .remove("executionExpiresAt");
    assert!(serde_json::from_value::<InstanceLease>(missing).is_err());
}
