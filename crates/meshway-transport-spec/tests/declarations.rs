use meshway_transport_spec::*;
use serde_json::json;
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
    for protocol in ["native_udp", "native_tcp"] {
        let source = json!({"type": protocol, "listenPort": 8421});
        let listener: Listener = serde_json::from_value(source.clone()).unwrap();
        assert!(listener.material_requirements().is_empty());
        assert!(PreparedListener::prepare(listener).is_ok());
        assert!(serde_json::from_value::<PreparedListener>(source).is_ok());
    }
}

#[test]
fn dialer_set_is_closed_and_omission_differs_from_disabled() {
    let d: DialerSet = serde_json::from_str(r#"{"vless":{}}"#).unwrap();
    assert!(d.vless.unwrap().enabled);
    assert!(d.native_udp.is_none());
    for text in [
        r#"{"vless":null}"#,
        r#"{"vless":{"priority":300}}"#,
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
    let prepared = PreparedListener::prepare(l.clone()).unwrap();
    let restored: PreparedListener =
        serde_json::from_value(serde_json::to_value(&prepared).unwrap()).unwrap();
    assert_eq!(prepared, restored);
    assert_eq!(
        restored.declaration().material_requirements(),
        vec![MaterialRequirement::CertificateValidation]
    );
    assert!(!format!("{l:?}").contains("SECRET"));
    let Listener::Vless(v) = &mut l else { panic!() };
    v.tls.alpn = vec!["h2".into()];
    assert!(l.normalize().is_err());
}

#[test]
fn prepared_tls_listeners_require_all_structural_material() {
    for (protocol, credential, value) in [
        ("vless", "uuid", "11111111-1111-4111-8111-111111111111"),
        ("trojan", "password", "password fixture"),
    ] {
        let source = json!({
            "type": protocol, "listenPort": 443, credential: value,
            "tls": {"serverName": "EDGE.Example.", "certificate": "certificate fixture",
                "privateKey": "key fixture"}
        });
        let prepared: PreparedListener = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&prepared).unwrap()["tls"]["serverName"],
            "edge.example"
        );
        assert_eq!(
            prepared.declaration().material_requirements(),
            vec![MaterialRequirement::CertificateValidation]
        );
        for field in [credential, "certificate", "privateKey"] {
            let mut incomplete = source.clone();
            let object = if field == credential {
                &mut incomplete
            } else {
                &mut incomplete["tls"]
            };
            object.as_object_mut().unwrap().remove(field);
            let listener = serde_json::from_value(incomplete.clone()).unwrap();
            assert!(
                PreparedListener::prepare(listener).is_err(),
                "{protocol}/{field}"
            );
            assert!(serde_json::from_value::<PreparedListener>(incomplete).is_err());
        }
    }
}

#[test]
fn websocket_header_debug_redacts_values_through_transport_declarations() {
    let source = json!({"Authorization": "Bearer auth-secret", "X-Custom": "custom-secret"});
    let headers: WebSocketHeaders = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        vec![
            ("Authorization", "Bearer auth-secret"),
            ("X-Custom", "custom-secret")
        ]
    );
    assert_eq!(serde_json::to_value(&headers).unwrap(), source);
    let value: WebSocketHeaderValue = "custom-secret".parse().unwrap();
    assert_eq!(value.as_str(), "custom-secret");
    assert_eq!(serde_json::to_value(&value).unwrap(), "custom-secret");

    let listener: Listener = serde_json::from_value(json!({
        "type": "vless", "listenPort": 443, "stream": "websocket",
        "uuid": "11111111-1111-4111-8111-111111111111",
        "tls": {"serverName": "edge.example", "certificate": "certificate fixture",
            "privateKey": "key fixture"},
        "websocket": {"responseHeaders": source}
    }))
    .unwrap();
    let prepared = PreparedListener::prepare(listener.clone()).unwrap();
    let dialers: DialerSet = serde_json::from_value(json!({
        "vless": {"stream": "websocket", "websocket": {"headers": source}}
    }))
    .unwrap();
    for debug in [
        format!("{value:?}"),
        format!("{value:#?}"),
        format!("{headers:?}"),
        format!("{headers:#?}"),
        format!("{listener:?}"),
        format!("{listener:#?}"),
        format!("{prepared:?}"),
        format!("{prepared:#?}"),
        format!("{dialers:?}"),
        format!("{dialers:#?}"),
    ] {
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("auth-secret"));
        assert!(!debug.contains("custom-secret"));
    }
    assert_eq!(
        serde_json::to_value(listener).unwrap()["websocket"]["responseHeaders"],
        source
    );
    assert_eq!(
        serde_json::to_value(dialers).unwrap()["vless"]["websocket"]["headers"],
        source
    );
}

#[test]
fn actual_listener_and_dialer_declarations_validate_tls_version_order() {
    for protocol in ["vless", "trojan"] {
        for (min, max, valid) in [
            ("1.2", "1.2", true),
            ("1.2", "1.3", true),
            ("1.3", "1.3", true),
            ("1.3", "1.2", false),
        ] {
            let mut listener: Listener = serde_json::from_value(json!({
                "type": protocol, "listenPort": 443,
                "tls": {"serverName": "edge.example", "minVersion": min, "maxVersion": max}
            }))
            .unwrap();
            assert_eq!(
                listener.normalize().is_ok(),
                valid,
                "{protocol} {min}..{max}"
            );
            let dialers: DialerSet = serde_json::from_value(json!({
                protocol: {"tls": {"minVersion": min, "maxVersion": max}}
            }))
            .unwrap();
            assert_eq!(dialers.validate().is_ok(), valid, "{protocol} {min}..{max}");
            let listener_json = serde_json::to_value(listener).unwrap();
            let dialer_json = serde_json::to_value(dialers).unwrap();
            for tls in [&listener_json["tls"], &dialer_json[protocol]["tls"]] {
                assert_eq!(tls["minVersion"], min);
                assert_eq!(tls["maxVersion"], max);
                assert!(tls.get("versions").is_none());
            }
        }
    }
}

#[test]
fn actual_tls_fields_reject_unknown_versions_coercion_and_duplicate_bounds() {
    for field in ["minVersion", "maxVersion"] {
        for invalid in [
            json!("1.1"),
            json!("1.4"),
            json!(1.2),
            json!(null),
            json!([]),
        ] {
            let mut listener = json!({"type": "vless", "listenPort": 443,
                "tls": {"serverName": "edge.example"}});
            listener["tls"][field] = invalid.clone();
            assert!(serde_json::from_value::<Listener>(listener).is_err());
            let dialers = json!({"vless": {"tls": {field: invalid}}});
            assert!(serde_json::from_value::<DialerSet>(dialers).is_err());
        }
    }
    assert!(serde_json::from_str::<Listener>(r#"{"type":"vless","listenPort":443,"tls":{"serverName":"edge.example","minVersion":"1.2","minVersion":"1.3"}}"#).is_err());
    assert!(
        serde_json::from_str::<DialerSet>(
            r#"{"vless":{"tls":{"maxVersion":"1.2","maxVersion":"1.3"}}}"#
        )
        .is_err()
    );
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
