use meshway_protocol::ipc::{
    Command, PingError, PingEvent, Request, Response, ResultBody, VERSION,
};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use utoipa::PartialSchema;

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn target() -> Value {
    json!({"nodeId":ID,"hostname":"node.mesh","addresses":{"ipv4":"10.0.0.1","ipv6":null}})
}

#[test]
fn ping_command_has_a_bounded_count_and_no_budget_override() {
    assert_eq!(VERSION, 8);
    for count in [1, 10, 1000] {
        let value = json!({"version":VERSION,"requestId":ID,"command":{"type":"ping","target":{"type":"name","name":"node.mesh"},"count":count}});
        let request: Request = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), value);
    }
    for invalid in [
        json!({"type":"ping","target":{"type":"meshIp","address":"10.0.0.1"},"count":0}),
        json!({"type":"ping","target":{"type":"meshIp","address":"10.0.0.1"},"count":1001}),
        json!({"type":"ping","target":{"type":"meshIp","address":"10.0.0.1"},"count":10,"timeout":5000}),
        json!({"type":"ping","target":{"type":"name","name":"node.mesh"}}),
        json!({"type":"ping","target":["name","node.mesh"],"count":10}),
    ] {
        assert!(serde_json::from_value::<Command>(invalid).is_err());
    }
    let schema = serde_json::to_value(Command::schema()).unwrap();
    let ping = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["properties"]["type"]["enum"] == json!(["ping"]))
        .unwrap();
    assert_eq!(ping["properties"]["count"]["minimum"], 1);
    assert_eq!(ping["properties"]["count"]["maximum"], 1000);
}

#[tokio::test]
async fn multiple_ping_events_use_the_existing_response_envelope_and_framing() {
    let events = [
        json!({"type":"accepted","subscriptionId":ID,"observationId":ID,"target":target()}),
        json!({"type":"preparing","elapsedMs":1000}),
        json!({"type":"sample","observationId":ID,"sample":{
            "measurementId":ID,"version":1,"windowStart":"2026-10-05T00:00:00Z",
            "windowEnd":"2026-10-05T00:00:05Z","partial":false,"probe":null,
            "traffic":null,"probeCounts":{"sent":1,"repliedWithinDeadline":0,"timeout":1,"pending":0}
        }}),
        json!({"type":"completed","sent":1,"success":0,"timeout":1,"failed":0}),
        json!({"type":"failed","code":"stopped"}),
    ];
    let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
    for (sequence, event) in events.iter().enumerate() {
        let encoded = json!({"version":VERSION,"requestId":ID,"result":{"type":"ping","sequence":sequence,"event":event}});
        let response: Response = serde_json::from_value(encoded.clone()).unwrap();
        meshway_protocol::ipc::write(&mut writer, &response)
            .await
            .unwrap();
        let decoded: Response = meshway_protocol::ipc::read(&mut reader).await.unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }
    let error =
        json!({"version":VERSION,"requestId":ID,"result":{"type":"error","code":"version"}});
    assert!(matches!(
        serde_json::from_value::<Response>(error).unwrap().result,
        ResultBody::Error { .. }
    ));
}

#[test]
fn nested_ping_payloads_are_closed_typed_objects() {
    for invalid in [
        json!({"type":"accepted","subscriptionId":ID,"observationId":ID,"target":[]}),
        json!({"type":"accepted","subscriptionId":ID,"observationId":ID,"target":target(),"extra":null}),
        json!({"type":"preparing","elapsedMs":-1}),
        json!({"type":"preparing","elapsedMs":9_007_199_254_740_992_u64}),
        json!({"type":"sample","observationId":ID,"sample":[]}),
        json!({"type":"completed","sent":1,"success":0,"timeout":1}),
        json!({"type":"failed","code":{"stopped":null}}),
    ] {
        assert!(serde_json::from_value::<PingEvent>(invalid).is_err());
    }
    assert!(
        serde_json::from_str::<PingEvent>(r#"{"type":"preparing","elapsedMs":0,"elapsedMs":1}"#)
            .is_err()
    );
    for code in [
        "notRunning",
        "unsupported",
        "permissionDenied",
        "targetNotFound",
        "timeout",
        "busy",
        "stopped",
        "protocol",
        "failed",
    ] {
        let error: PingError = serde_json::from_value(json!(code)).unwrap();
        assert_eq!(serde_json::to_value(error).unwrap(), json!(code));
    }
    assert!(serde_json::from_value::<PingError>(json!("unknown")).is_err());
}

fn status() -> Value {
    json!({
        "processId":1,"revision":1,"desiredUp":false,"phase":"down","server":"https://server.test",
        "sync":"disconnected","lastSyncAt":null,"syncError":null,"netmap":null,
        "observedAt":"2026-10-05T00:00:00Z","enrollmentRequestId":null,"enrollmentUrl":null,
        "identity":null,"authenticated":false,"instance":null,"saved":null,"applied":null,
        "dataPlane":"stopped","networkMode":"tun","lastError":null,"retryAt":null,
        "node":null,"peers":[]
    })
}

#[tokio::test]
async fn status_stays_object_only_through_tagged_buffering_and_nullable_fields_are_required() {
    let status = status();
    let good = json!({"version":VERSION,"requestId":ID,"result":{"type":"status","status":status}});
    assert!(serde_json::from_value::<Response>(good.clone()).is_ok());
    // A complete positional Status was previously accepted after enum buffering.
    let positional = json!([
        1,
        1,
        false,
        "down",
        "https://server.test",
        "2026-10-05T00:00:00Z",
        null,
        null,
        null,
        false,
        "disconnected",
        null,
        null,
        null,
        null,
        "stopped",
        "tun",
        null,
        null,
        [],
        null,
        null
    ]);
    let mut invalid = good.clone();
    invalid["result"]["status"] = positional;
    let bytes = serde_json::to_vec(&invalid).unwrap();
    let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
    writer.write_u32(bytes.len() as u32).await.unwrap();
    writer.write_all(&bytes).await.unwrap();
    assert!(
        meshway_protocol::ipc::read::<Response>(&mut reader)
            .await
            .is_err()
    );
    for field in [
        "enrollmentRequestId",
        "enrollmentUrl",
        "identity",
        "instance",
        "saved",
        "applied",
        "lastError",
        "retryAt",
        "sync",
        "lastSyncAt",
        "syncError",
        "netmap",
        "node",
        "peers",
    ] {
        let mut missing = good.clone();
        missing["result"]["status"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<Response>(missing).is_err(),
            "{field}"
        );
    }
    for (field, invalid) in [
        ("phase", json!({"down":null})),
        ("dataPlane", json!({"stopped":null})),
        ("networkMode", json!({"tun":null})),
        ("lastError", json!({"network":null})),
        ("eventStream", json!("unknown")),
    ] {
        let mut malformed = good.clone();
        malformed["result"]["status"][field] = invalid;
        assert!(
            serde_json::from_value::<Response>(malformed).is_err(),
            "{field}"
        );
    }
    assert!(
        serde_json::from_value::<ResultBody>(json!({"type":"error","code":{"busy":null}})).is_err()
    );
}
