use meshway_protocol::{
    CreateNodeObservation, NodeObservationCancelled, NodeObservationQuery, NodeObservationResponse,
    NodeObservationTarget, NodeObservationTargetView, RenewNodeObservation,
};
use serde_json::{Value, json};
use utoipa::PartialSchema;

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn create() -> Value {
    json!({
        "instance":{"instanceId":ID,"generation":1},"subscriptionId":ID,
        "target":{"type":"name","name":"node.mesh"},"service":{"type":"icmpEcho"},
        "selection":{"mode":"auto"},"metrics":["rtt","probeLoss"],"intervalMs":1000
    })
}

#[test]
fn selectors_remain_strings_with_matching_runtime_and_schema_length_bounds() {
    for (kind, field, limit) in [("name", "name", 253), ("meshIp", "address", 64)] {
        for value in [
            "".into(),
            "not a validated address".into(),
            "é".repeat(limit),
        ] {
            let encoded = json!({"type":kind,field:value});
            let target: NodeObservationTarget = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(serde_json::to_value(target).unwrap(), encoded);
        }
        let oversized = json!({"type":kind,field:"é".repeat(limit+1)});
        assert!(serde_json::from_value::<NodeObservationTarget>(oversized).is_err());
        for value in [json!(null), json!(1), json!([])] {
            assert!(
                serde_json::from_value::<NodeObservationTarget>(json!({"type":kind,field:value}))
                    .is_err()
            );
        }
    }
    for invalid in [
        json!({"type":"name","name":"node.mesh","address":"10.0.0.1"}),
        json!({"type":"meshIp","address":"10.0.0.1","extra":null}),
        json!(["name", "node.mesh"]),
    ] {
        assert!(serde_json::from_value::<NodeObservationTarget>(invalid).is_err());
    }
    assert!(
        serde_json::from_str::<NodeObservationTarget>(r#"{"type":"name","name":"a","name":"b"}"#)
            .is_err()
    );
    let schema = serde_json::to_value(NodeObservationTarget::schema()).unwrap();
    assert_eq!(schema["oneOf"][0]["properties"]["name"]["maxLength"], 253);
    assert_eq!(schema["oneOf"][1]["properties"]["address"]["maxLength"], 64);
}

#[test]
fn node_create_preserves_routing_service_and_exact_instance_binding() {
    let encoded = create();
    let request: CreateNodeObservation = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), encoded);
    for (field, invalid) in [
        ("instance", json!([ID, 1])),
        ("target", json!(["name", "node.mesh"])),
        ("service", json!("icmpEcho")),
        ("selection", json!("auto")),
        ("intervalMs", json!(999)),
        ("intervalMs", json!(30001)),
    ] {
        let mut value = create();
        value[field] = invalid;
        assert!(
            serde_json::from_value::<CreateNodeObservation>(value).is_err(),
            "{field}"
        );
    }
    for field in create().as_object().unwrap().keys() {
        let mut value = create();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<CreateNodeObservation>(value).is_err(),
            "{field}"
        );
    }
    let query: NodeObservationQuery =
        serde_json::from_value(json!({"instanceId":ID,"generation":1})).unwrap();
    assert_eq!(query.generation.get(), 1);
    assert!(
        serde_json::from_value::<NodeObservationQuery>(json!({"instanceId":ID,"generation":0}))
            .is_err()
    );
}

#[test]
fn renewal_uses_an_absolute_millisecond_timestamp() {
    let encoded =
        json!({"instance":{"instanceId":ID,"generation":1},"expiresAt":"2026-10-05T00:00:00.001Z"});
    let renew: RenewNodeObservation = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(renew).unwrap(), encoded);
    for invalid in [
        json!(5000),
        json!("2026-10-05T00:00:00.000001Z"),
        json!(null),
    ] {
        let mut value = encoded.clone();
        value["expiresAt"] = invalid;
        assert!(serde_json::from_value::<RenewNodeObservation>(value).is_err());
    }
}

#[test]
fn cancelled_task_and_target_address_families_are_explicitly_nullable() {
    let cancelled: NodeObservationCancelled = serde_json::from_value(json!({"task":null})).unwrap();
    assert!(cancelled.task.is_none());
    assert_eq!(
        serde_json::to_value(cancelled).unwrap(),
        json!({"task":null})
    );
    for invalid in [
        json!({}),
        json!({"task":[]}),
        json!({"task":null,"cancelled":true}),
    ] {
        assert!(serde_json::from_value::<NodeObservationCancelled>(invalid).is_err());
    }
    let schema = serde_json::to_value(NodeObservationCancelled::schema()).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("task"))
    );
    let target =
        json!({"nodeId":ID,"hostname":"node.mesh","addresses":{"ipv4":"10.0.0.1","ipv6":null}});
    let view: NodeObservationTargetView = serde_json::from_value(target.clone()).unwrap();
    assert_eq!(serde_json::to_value(view).unwrap(), target);
    let mut missing_family = target;
    missing_family["addresses"]
        .as_object_mut()
        .unwrap()
        .remove("ipv6");
    assert!(serde_json::from_value::<NodeObservationTargetView>(missing_family).is_err());
}

#[test]
fn node_response_and_cancellation_retain_the_actual_shared_task() {
    let task = json!({
        "id":ID,"sourceNodeId":ID,"targetNodeId":ID,"service":{"type":"icmpEcho"},
        "selection":{"mode":"auto"},"path":null,"pathSegment":null,
        "netmap":{"revision":1,"serial":1,"digest":"01".repeat(32)},
        "instanceId":ID,"metrics":["rtt"],"intervalMs":1000,"phase":"preparing",
        "unsupportedMetrics":[],"expiresAt":"2026-10-05T00:00:30Z",
        "renewAfter":"2026-10-05T00:00:15Z","latest":null,
        "subscriberExpiresAt":null,"subscriptionId":null
    });
    let target =
        json!({"nodeId":ID,"hostname":"node.mesh","addresses":{"ipv4":"10.0.0.1","ipv6":null}});
    let value = json!({"task":task,"target":target});
    let response: NodeObservationResponse = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), value);
    let cancellation = json!({"task":task});
    let response: NodeObservationCancelled = serde_json::from_value(cancellation.clone()).unwrap();
    assert!(response.task.is_some());
    assert_eq!(serde_json::to_value(response).unwrap(), cancellation);
    for field in ["task", "target"] {
        let mut malformed = value.clone();
        malformed[field] = json!([]);
        assert!(serde_json::from_value::<NodeObservationResponse>(malformed).is_err());
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<NodeObservationResponse>(missing).is_err());
    }
}
