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

#[test]
fn enrollment_pagination_and_approval_distinguish_null_from_missing() {
    use meshway_protocol::enrollment::{DeviceAuthorizationView, EnrollmentKeysResponse};
    let page = json!({"items": [], "total": 0, "nextCursor": null, "asOf": "2026-10-03T00:00:00Z"});
    assert!(serde_json::from_value::<EnrollmentKeysResponse>(page.clone()).is_ok());
    let mut missing_cursor = page.clone();
    missing_cursor.as_object_mut().unwrap().remove("nextCursor");
    assert!(serde_json::from_value::<EnrollmentKeysResponse>(missing_cursor).is_err());
    let mut oversized = page;
    oversized["total"] = json!(9_007_199_254_740_992_u64);
    assert!(serde_json::from_value::<EnrollmentKeysResponse>(oversized).is_err());

    let mut view = json!({
        "request": {
            "id": "00000000-0000-4000-8000-000000000001",
            "hostname": "device", "controlKey": {"algorithm": "ed25519", "bytes": "11".repeat(32)},
            "existing": null, "returnMethod": "poll", "phase": "pending",
            "expiresAt": "2026-10-03T01:00:00Z", "authorizationUrl": null
        },
        "approval": null
    });
    assert!(serde_json::from_value::<DeviceAuthorizationView>(view.clone()).is_ok());
    let mut no_link = view.clone();
    no_link["request"]
        .as_object_mut()
        .unwrap()
        .remove("authorizationUrl");
    assert!(serde_json::from_value::<DeviceAuthorizationView>(no_link).is_err());
    view.as_object_mut().unwrap().remove("approval");
    assert!(serde_json::from_value::<DeviceAuthorizationView>(view).is_err());
}
