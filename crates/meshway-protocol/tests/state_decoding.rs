use meshway_protocol::NodeEvent;
use serde_json::json;

#[test]
fn state_snapshot_rejects_resource_shape_and_range_errors_after_multiple_tagged_layers() {
    let id = "00000000-0000-4000-8000-000000000001";
    let time = "2026-10-04T00:00:00Z";
    let snapshot = json!({
        "type":"state.snapshot", "batch":"one",
        "state":{
            "networkId":id,"stateEpoch":id,"sequence":1,"configurationRevision":1,
            "asOf":time,"links":[],"paths":[],
            "nodes":{(id):{
                "hostname":"node","freshness":"current",
                "derived":{"ipInfo":{"198.51.100.1":{
                    "geoip":{"status":"available","datasetId":id,"lookedUpAt":time,
                        "data":{"country":"US","position":{"latitude":45,"longitude":90}}},
                    "asn":{"status":"available","datasetId":id,"lookedUpAt":time,
                        "data":{"number":64500}}
                }}}
            }}
        }
    });
    assert!(serde_json::from_value::<NodeEvent>(snapshot.clone()).is_ok());
    let root = format!("/state/nodes/{id}/derived/ipInfo/198.51.100.1");
    for (suffix, replacement) in [
        ("/geoip/data", json!(["US", null, null, null])),
        ("/geoip/data/position", json!([45, 90, null])),
        ("/geoip/data/position/latitude", json!(91)),
        ("/asn/data", json!([64500, null, null])),
    ] {
        let mut malformed = snapshot.clone();
        *malformed.pointer_mut(&format!("{root}{suffix}")).unwrap() = replacement;
        assert!(
            serde_json::from_value::<NodeEvent>(malformed).is_err(),
            "{suffix}"
        );
    }
}
