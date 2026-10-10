use meshway_configuration::{ListenerUpdate, inherit_materials};
use meshway_transport::spec::Listener;

#[test]
fn managed_certificates_are_reissued_per_server_name_and_provided_ones_are_kept() {
    let previous: Listener = serde_json::from_str(
        r#"{"type":"vless","listenPort":443,"uuid":"11111111-1111-4111-8111-111111111111",
            "tls":{"serverName":"a.example.test","certificate":"CERT-A","privateKey":"KEY-A"}}"#,
    )
    .unwrap();
    let update = |tls: &str| -> Listener {
        serde_json::from_str(&format!(
            r#"{{"type":"vless","listenPort":8443,"tls":{tls}}}"#
        ))
        .unwrap()
    };
    let mut same = update(r#"{"serverName":"A.example.test"}"#);
    inherit_materials(&mut same, &previous);
    let Listener::Vless(v) = &same else { panic!() };
    assert_eq!(
        v.uuid.as_ref().map(|s| s.expose()),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(
        v.tls.certificate.as_ref().map(|s| s.expose()),
        Some("CERT-A")
    );
    // A managed certificate is for one name; a new name gets a new one.
    let mut renamed = update(r#"{"serverName":"b.example.test"}"#);
    inherit_materials(&mut renamed, &previous);
    let Listener::Vless(v) = &renamed else {
        panic!()
    };
    assert_eq!(
        v.uuid.as_ref().map(|s| s.expose()),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(v.tls.certificate, None);
    assert_eq!(v.tls.private_key, None);
    // Provided material is the author's: omission keeps it regardless of name.
    let provided: Listener = serde_json::from_str(
        r#"{"type":"vless","listenPort":443,"uuid":"11111111-1111-4111-8111-111111111111",
            "tls":{"mode":"provided","serverName":"a.example.test","certificate":"CERT-A","privateKey":"KEY-A"}}"#,
    ).unwrap();
    let mut renamed = update(r#"{"mode":"provided","serverName":"b.example.test"}"#);
    inherit_materials(&mut renamed, &provided);
    let Listener::Vless(v) = &renamed else {
        panic!()
    };
    assert_eq!(
        v.tls.certificate.as_ref().map(|s| s.expose()),
        Some("CERT-A")
    );
}

#[test]
fn listener_updates_keep_omitted_websocket_response_headers_for_the_same_resource() {
    let previous: Listener = serde_json::from_str(
        r#"{"type":"vless","listenPort":443,"stream":"websocket","uuid":"11111111-1111-4111-8111-111111111111",
            "tls":{"serverName":"a.example.test","alpn":["http/1.1"],"certificate":"CERT-A","privateKey":"KEY-A"},
            "websocket":{"path":"/ws","responseHeaders":{"X-Keep":"yes"}}}"#,
    ).unwrap();
    let update = |websocket: &str| -> ListenerUpdate {
        serde_json::from_str(&format!(r#"{{"type":"vless","listenPort":8443,"stream":"websocket","tls":{{"serverName":"a.example.test","alpn":["http/1.1"]}}{websocket}}}"#)).unwrap()
    };
    let headers = |l: &Listener| match l {
        Listener::Vless(v) => v
            .websocket
            .as_ref()
            .map(|w| serde_json::to_value(&w.response_headers).unwrap()),
        _ => None,
    };
    // Omitted websocket block: the previous block, headers included.
    assert_eq!(
        headers(&update("").apply(Some(&previous))),
        headers(&previous)
    );
    // Websocket block without headers: the previous headers.
    assert_eq!(
        headers(&update(r#","websocket":{"path":"/new"}"#).apply(Some(&previous))),
        headers(&previous)
    );
    // Explicitly empty headers clear them; an explicit null removes the block.
    assert_eq!(
        headers(
            &update(r#","websocket":{"path":"/new","responseHeaders":{}}"#).apply(Some(&previous))
        ),
        Some(serde_json::json!({}))
    );
    assert_eq!(
        headers(&update(r#","websocket":null"#).apply(Some(&previous))),
        None
    );
    // A new resource has no predecessor to inherit from.
    assert_eq!(
        headers(&update(r#","websocket":{"path":"/new"}"#).apply(None)),
        Some(serde_json::json!({}))
    );
    let Listener::Vless(v) = update("").apply(Some(&previous)) else {
        panic!()
    };
    assert_eq!(
        v.uuid.as_ref().map(|s| s.expose()),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(
        v.tls.certificate.as_ref().map(|s| s.expose()),
        Some("CERT-A")
    );
}
