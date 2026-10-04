use meshway_protocol::enrollment::EnrollmentIntent;
use serde_json::json;
use utoipa::PartialSchema;

#[test]
fn signed_intent_requires_explicit_nullable_existing_binding() {
    let mut input = json!({
        "id": "00000000-0000-4000-8000-000000000001",
        "hostname": "device",
        "controlKey": {"algorithm": "ed25519", "bytes": "11".repeat(32)},
        "dataplaneKey": {"algorithm": "x25519", "bytes": "22".repeat(32)},
        "codeChallenge": "c".repeat(43)
    });
    assert!(serde_json::from_value::<EnrollmentIntent>(input.clone()).is_err());
    input["existing"] = serde_json::Value::Null;
    let decoded: EnrollmentIntent = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), input);
    input["existing"] = json!({
        "networkId": "00000000-0000-4000-8000-000000000002",
        "nodeId": "00000000-0000-4000-8000-000000000003",
        "keyId": "00000000-0000-4000-8000-000000000004",
        "keyGeneration": 1
    });
    let decoded: EnrollmentIntent = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), input);
    let schema = serde_json::to_value(EnrollmentIntent::schema()).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("existing"))
    );
}
