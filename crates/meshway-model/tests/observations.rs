use meshway_model::{
    observations::{EndpointKind, LinkAcquisition, ProbeChannel, ProbeChannels},
    state::ProbeObservation,
};
use serde_json::{Value, json};
use utoipa::PartialSchema;

const ID: &str = "00000000-0000-4000-8000-000000000001";

fn channel() -> Value {
    json!({
        "linkId": "01".repeat(32), "socketId": "02".repeat(32), "peerNodeId": ID,
        "transport": "native_udp", "localAddress": "192.0.2.1:31000",
        "remoteAddress": "198.51.100.2:32000",
        "acquisition": {"type":"coordinatedUdp", "attemptId":ID, "endpointSource":"stun"}
    })
}

#[test]
fn physical_channels_roundtrip_without_disclosing_private_origin_details() {
    let request = channel();
    let mut reply = channel();
    reply["linkId"] = json!("03".repeat(32));
    reply["remoteAddress"] = json!("198.51.100.2:32001");
    reply["acquisition"] = json!({"type":"acceptedInbound"});
    let encoded = json!({"request":request,"reply":reply});
    let channels: ProbeChannels = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(channels).unwrap(), encoded);
    for acquisition in [
        json!({"type":"publishedEndpoint"}),
        json!({"type":"acceptedInbound"}),
        json!({"type":"coordinatedUdp","attemptId":ID,"endpointSource":"interface"}),
    ] {
        assert!(serde_json::from_value::<LinkAcquisition>(acquisition).is_ok());
    }
    for (field, invalid) in [
        ("linkId", json!("unverified-link")),
        ("socketId", json!("socket-name")),
        ("transport", json!("unknown")),
        ("remoteAddress", json!("peer.test:53")),
        (
            "acquisition",
            json!({"type":"coordinatedUdp","attemptId":ID,"endpointSource":"stun","observer":"private"}),
        ),
        (
            "acquisition",
            json!({"type":"coordinatedUdp","attemptId":ID,"endpointSource":{"stun":null}}),
        ),
    ] {
        let mut invalid_channel = channel();
        invalid_channel[field] = invalid;
        assert!(
            serde_json::from_value::<ProbeChannel>(invalid_channel).is_err(),
            "{field}"
        );
    }
    for invalid in [json!("Stun"), json!({"stun":null}), json!(["stun"])] {
        assert!(serde_json::from_value::<EndpointKind>(invalid).is_err());
    }
}

#[test]
fn missing_channel_measurement_and_explicitly_absent_reply_are_distinct() {
    let value = json!({"request":channel(),"reply":null});
    assert!(
        serde_json::from_value::<ProbeChannels>(value.clone())
            .unwrap()
            .reply
            .is_none()
    );
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<ProbeChannels>(value.clone()).unwrap())
            .unwrap(),
        value
    );
    assert!(serde_json::from_value::<ProbeChannels>(json!({"request":channel()})).is_err());
    assert!(serde_json::from_value::<ProbeChannels>(json!({"request":null,"reply":null})).is_err());
    assert!(serde_json::from_value::<ProbeChannels>(json!({"request":[],"reply":null})).is_err());
    let schema = serde_json::to_value(ProbeChannels::schema()).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("reply"))
    );

    let mut probe = json!({
        "measurementId":ID,"kind":"path","linkId":null,"transport":null,
        "path":[ID],"netmap":{"revision":1,"serial":1,"digest":"01".repeat(32)},"startedAt":"2026-10-05T00:00:00Z",
        "completedAt":null,"outcome":"timeout","durationUs":null
    });
    assert!(
        serde_json::from_value::<ProbeObservation>(probe.clone())
            .unwrap()
            .channels
            .is_none()
    );
    probe["channels"] = Value::Null;
    assert!(
        serde_json::from_value::<ProbeObservation>(probe.clone())
            .unwrap()
            .channels
            .is_none()
    );
    probe["channels"] = value;
    let decoded: ProbeObservation = serde_json::from_value(probe).unwrap();
    assert!(decoded.channels.is_some());
    assert!(decoded.duration_us.is_none());
    assert!(decoded.link_id.is_none());
    assert!(decoded.transport.is_none());
}
