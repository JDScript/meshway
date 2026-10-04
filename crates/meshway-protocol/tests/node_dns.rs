use meshway_protocol::node::{NodeDnsQueryRequest, NodeDnsQueryResponse};
use serde_json::json;
use utoipa::PartialSchema;

#[test]
fn discovery_contract_is_exact_and_requires_explicit_nullable_record_and_families() {
    for request in [
        json!({}),
        json!({"name":null}),
        json!(["node.mesh.test"]),
        json!({"name":"node.mesh.test", "sourceNodeId":"00000000-0000-4000-8000-000000000001"}),
        json!({"name":"node.mesh.test", "networkId":"00000000-0000-4000-8000-000000000001"}),
        json!({"name":"node.mesh.test", "names":[]}),
    ] {
        assert!(serde_json::from_value::<NodeDnsQueryRequest>(request).is_err());
    }
    assert!(serde_json::from_str::<NodeDnsQueryRequest>(r#"{"name":"a","name":"b"}"#).is_err());
    let mut response = json!({
        "requestId":"00000000-0000-4000-8000-000000000001",
        "networkId":"00000000-0000-4000-8000-000000000002",
        "sourceNodeId":"00000000-0000-4000-8000-000000000003",
        "configurationRevision":1, "asOf":"2026-10-04T00:00:00Z", "record":null
    });
    assert!(
        serde_json::from_value::<NodeDnsQueryResponse>(response.clone())
            .unwrap()
            .record
            .is_none()
    );
    response.as_object_mut().unwrap().remove("record");
    assert!(serde_json::from_value::<NodeDnsQueryResponse>(response.clone()).is_err());
    response["record"] = json!({
        "nodeId":"00000000-0000-4000-8000-000000000004", "name":"node.mesh.test",
        "addresses":{"ipv4":null,"ipv6":null}
    });
    let decoded = serde_json::from_value::<NodeDnsQueryResponse>(response.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), response);
    response["record"]["addresses"]
        .as_object_mut()
        .unwrap()
        .remove("ipv6");
    assert!(serde_json::from_value::<NodeDnsQueryResponse>(response).is_err());
    let schema = serde_json::to_value(NodeDnsQueryResponse::schema()).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("record"))
    );
}
