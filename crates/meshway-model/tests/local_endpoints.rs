use meshway_model::state::{LocalEndpoint, SocketState};
use serde_json::json;

#[test]
fn local_listener_sample_distinguishes_omission_from_known_empty_and_requires_provenance() {
    let mut wire = json!({
        "resource": {"kind": "listener", "key": "udp"},
        "configuration": null, "generation": 1, "network": "udp",
        "boundAddress": "0.0.0.0:35000", "endpoints": []
    });
    let unknown: SocketState = serde_json::from_value(wire.clone()).unwrap();
    assert!(unknown.local_endpoints.is_none());
    assert!(
        serde_json::to_value(unknown)
            .unwrap()
            .get("localEndpoints")
            .is_none()
    );
    wire["localEndpoints"] = json!([]);
    let empty: SocketState = serde_json::from_value(wire).unwrap();
    assert_eq!(empty.local_endpoints, Some(vec![]));
    for value in [
        json!({"address": "10.0.0.1:35000"}),
        json!({"address": "10.0.0.1:35000", "source": "configured"}),
        json!({"address": "example.com:35000", "source": "local"}),
    ] {
        assert!(serde_json::from_value::<LocalEndpoint>(value).is_err());
    }
}
