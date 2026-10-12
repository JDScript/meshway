use meshway_protocol::UserEvent;
use serde_json::json;

#[test]
fn state_snapshot_rejects_derived_and_adjacency_shape_errors_after_multiple_tagged_layers() {
    let id = "00000000-0000-4000-8000-000000000001";
    let peer = "00000000-0000-4000-8000-000000000002";
    let time = "2026-10-04T00:00:00Z";
    let snapshot = json!({
        "type":"state.snapshot", "batch":"one",
        "state":{
            "networkId":id,"stateEpoch":id,"sequence":1,"configurationRevision":1,
            "asOf":time,"links":[],"paths":[],
            "adjacencies":[{"a":id,"b":peer,"status":"connected"}],
            "nodes":{(id):{
                "hostname":"node","freshness":"current",
                "derived":{
                    "addressScope":{"198.51.100.1":"public","10.0.0.1":"nonPublic"},
                    "location":{"address":"198.51.100.1","endpointSource":"stun",
                        "observedAt":time,"expiresAt":time,
                        "sourceSocketId":"s1","sourceGeneration":1},
                    "locationAbsent":null
                }
            }}
        }
    });
    assert!(serde_json::from_value::<UserEvent>(snapshot.clone()).is_ok());
    let derived = format!("/state/nodes/{id}/derived");
    for (pointer, replacement) in [
        (format!("{derived}/addressScope/198.51.100.1"), json!("geo")),
        (format!("{derived}/location"), json!(["198.51.100.1"])),
        (format!("{derived}/location/sourceGeneration"), json!(-1)),
        (
            format!("{derived}/locationAbsent"),
            json!("datasetUnavailable"),
        ),
        ("/state/adjacencies/0/status".to_owned(), json!("oneSided")),
    ] {
        let mut malformed = snapshot.clone();
        *malformed.pointer_mut(&pointer).unwrap() = replacement;
        assert!(
            serde_json::from_value::<UserEvent>(malformed).is_err(),
            "{pointer}"
        );
    }
    // Retired GeoIP fields are unknown, not ignored.
    let mut malformed = snapshot.clone();
    malformed
        .pointer_mut(&format!("{derived}/location"))
        .unwrap()["datasetId"] = json!(id);
    assert!(serde_json::from_value::<UserEvent>(malformed).is_err());
    let mut malformed = snapshot;
    malformed.pointer_mut(&derived).unwrap()["ipInfo"] = json!({});
    assert!(serde_json::from_value::<UserEvent>(malformed).is_err());
}
