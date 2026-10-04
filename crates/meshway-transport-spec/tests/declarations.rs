use meshway_transport_spec::*;
use utoipa::PartialSchema;

#[test]
fn listener_protocols_have_distinct_closed_field_sets() {
    for json in [
        r#"{"type":"native_udp","listenPort":8421,"tcp":{}}"#,
        r#"{"type":"native_tcp","listenPort":8421,"tls":{}}"#,
        r#"{"type":"trojan","listenPort":443,"stream":"websocket","tls":{"serverName":"edge.example"}}"#,
        r#"{"type":"vless","listenPort":443,"reality":{},"tls":{"serverName":"edge.example"}}"#,
        r#"{"type":"vless","listenPort":443,"uuid":null,"tls":{"serverName":"edge.example"}}"#,
        r#"{"type":"native_udp","listenPort":8421,"listenPort":8422}"#,
        r#"{"type":"native_udp","type":"native_tcp","listenPort":8421}"#,
        r#"{"type":"native_udp","listenPort":8421,"socket":[]}"#,
        r#"{"type":"native_udp","listenPort":0}"#,
        r#"{"type":"shadowsocks","listenPort":8421}"#,
    ] {
        assert!(
            serde_json::from_str::<Listener>(json).is_err(),
            "accepted {json}"
        );
    }
    let listener: Listener =
        serde_json::from_str(r#"{"type":"native_udp","listenPort":8421}"#).unwrap();
    assert!(PreparedListener::prepare(listener, false).is_ok());
}

#[test]
fn dialer_set_is_closed_and_omission_differs_from_disabled() {
    let d: DialerSet = serde_json::from_str(r#"{"vless":{}}"#).unwrap();
    assert_eq!(d.vless.unwrap().priority, 300);
    for text in [
        r#"{"vless":null}"#,
        r#"{"vless":{"type":"vless"}}"#,
        r#"{"native_udp":{"tcp":{}}}"#,
        r#"{"vless":{"streams":["tcp"]}}"#,
        r#"{"vless":{},"vless":{}}"#,
        r#"{"trojan":{"tls":{"trust":{"managed":null}}}}"#,
    ] {
        assert!(serde_json::from_str::<DialerSet>(text).is_err(), "{text}");
    }
    let d: DialerSet =
        serde_json::from_str(r#"{"vless":{"stream":"tcp","websocket":{}}}"#).unwrap();
    assert!(d.validate().is_err());
}

#[test]
fn tls_and_websocket_are_normalized_without_faking_material_verification() {
    let mut l: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"stream":"websocket","uuid":"11111111-1111-4111-8111-111111111111","tls":{"serverName":"EDGE.Example.","certificate":"SECRET CERTIFICATE","privateKey":"SECRET KEY"}}"#).unwrap();
    l.normalize().unwrap();
    let Listener::Vless(v) = &l else { panic!() };
    assert_eq!(
        v.websocket.as_ref().unwrap().host.as_deref(),
        Some("edge.example")
    );
    assert_eq!(v.websocket.as_ref().unwrap().path.as_str(), "/meshway");
    assert_eq!(
        l.material_requirements(),
        vec![MaterialRequirement::CertificateValidation]
    );
    assert!(PreparedListener::prepare(l.clone(), false).is_err());
    assert!(!format!("{l:?}").contains("SECRET"));
    let Listener::Vless(v) = &mut l else { panic!() };
    v.tls.alpn = vec!["h2".into()];
    assert!(l.normalize().is_err());
}

#[test]
fn declaration_schema_preserves_objects_and_closed_protocol_options() {
    let schema = serde_json::to_value(DialerSet::schema()).unwrap();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("vless").is_some());
    assert!(schema["properties"].get("shadowsocks").is_none());
    assert_eq!(
        serde_json::to_value(NativeUdpListener::schema()).unwrap()["additionalProperties"],
        false
    );
    assert!(
        serde_json::to_value(NativeUdpListener::schema()).unwrap()["properties"]
            .get("tcp")
            .is_none()
    );
}
