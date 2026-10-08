use meshway_transport_spec::*;
use serde_json::json;
use utoipa::{PartialSchema, ToSchema};

#[test]
fn dial_parameters_preserve_wire_variants_and_nullable_websocket() {
    let tls = json!({"serverName": "edge.example", "certificateChain": "certificate fixture",
        "options": {}});
    for (source, protocol) in [
        (
            json!({"type": "nativeUdp", "options": {}}),
            TransportProtocol::NativeUdp,
        ),
        (
            json!({"type": "nativeTcp", "options": {}}),
            TransportProtocol::NativeTcp,
        ),
        (
            json!({"type": "vless", "options": {}, "stream": "tcp", "flow": "",
            "uuid": "11111111-1111-4111-8111-111111111111", "tls": tls,
            "websocket": null}),
            TransportProtocol::Vless,
        ),
        (
            json!({"type": "trojan", "options": {}, "password": "password fixture",
            "tls": tls}),
            TransportProtocol::Trojan,
        ),
    ] {
        let parameters: DialParameters = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(parameters.transport(), protocol);
        let serialized = serde_json::to_value(&parameters).unwrap();
        assert_eq!(serialized["type"], source["type"]);
        assert_eq!(
            serde_json::from_value::<DialParameters>(serialized.clone()).unwrap(),
            parameters
        );
        if let DialParameters::Vless { tls, websocket, .. } = parameters {
            assert!(websocket.is_none());
            assert_eq!(tls.server_name, "edge.example");
            assert_eq!(tls.certificate_chain.expose(), "certificate fixture");
            assert_eq!(serialized["websocket"], json!(null));
            let mut omitted = source;
            omitted.as_object_mut().unwrap().remove("websocket");
            assert_eq!(
                serde_json::to_value(serde_json::from_value::<DialParameters>(omitted).unwrap())
                    .unwrap(),
                serialized
            );
        }
    }
}

#[test]
fn dial_parameters_keep_strict_nested_transport_values() {
    let source = json!({
        "type": "vless", "options": {"stream": "websocket"},
        "stream": "websocket", "flow": "", "uuid": "uuid fixture",
        "tls": {"serverName": "edge.example", "certificateChain": "certificate fixture", "options": {}},
        "websocket": {"path": "/meshway", "responseHeaders": {"Authorization": "Bearer auth-secret"}}
    });
    let parameters: DialParameters = serde_json::from_value(source.clone()).unwrap();
    assert!(!format!("{parameters:?}").contains("auth-secret"));
    assert!(!format!("{parameters:#?}").contains("auth-secret"));
    // Serialization makes defaulted fields explicit before replacing one.
    let serialized = serde_json::to_value(parameters).unwrap();
    assert_eq!(
        serialized["websocket"]["responseHeaders"]["Authorization"],
        "Bearer auth-secret"
    );
    for (path, invalid) in [
        ("/options", json!([])),
        ("/options/connectTimeout", json!("0s")),
        ("/options/socket", json!([])),
        ("/tls", json!(null)),
        ("/tls/options", json!(null)),
        ("/tls/options/minVersion", json!("1.1")),
        ("/tls/certificateChain", json!(null)),
        ("/websocket/path", json!("/\r\n")),
        ("/websocket/responseHeaders", json!({"Host": "evil"})),
        (
            "/websocket/responseHeaders",
            json!({"X-Test": "secret\r\n"}),
        ),
        ("/stream", json!("auto")),
    ] {
        let mut candidate = serialized.clone();
        *candidate.pointer_mut(path).unwrap() = invalid;
        assert!(
            serde_json::from_value::<DialParameters>(candidate).is_err(),
            "{path}"
        );
    }
    for field in ["serverName", "certificateChain", "options"] {
        let mut tls = source["tls"].clone();
        tls.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ClientTls>(tls).is_err(), "{field}");
    }
    let mut unknown = source;
    unknown["tls"]["insecure"] = json!(true);
    assert!(serde_json::from_value::<DialParameters>(unknown).is_err());
    assert!(serde_json::from_str::<ClientTls>(r#"{"serverName":"edge.example","serverName":"other.example","certificateChain":"fixture","options":{}}"#).is_err());
    assert!(
        serde_json::from_value::<DialParameters>(json!({"type": "native_udp", "options": {}}))
            .is_err()
    );
}

#[test]
fn prepared_dial_schemas_keep_names_references_and_nullable_fields() {
    assert_eq!(ClientTls::name(), "ClientTls");
    assert_eq!(DialParameters::name(), "DialParameters");
    let tls = serde_json::to_value(ClientTls::schema()).unwrap();
    assert_eq!(
        tls["required"],
        json!(["serverName", "certificateChain", "options"])
    );
    assert_eq!(tls["additionalProperties"], false);
    assert_eq!(
        tls["properties"]["certificateChain"]["$ref"],
        "#/components/schemas/SecretString"
    );
    assert_eq!(
        tls["properties"]["options"]["$ref"],
        "#/components/schemas/DialerTls"
    );
    let parameters = serde_json::to_value(DialParameters::schema()).unwrap();
    let variants = parameters["oneOf"].as_array().unwrap();
    for (variant, tag) in variants
        .iter()
        .zip(["nativeUdp", "nativeTcp", "vless", "trojan"])
    {
        assert_eq!(variant["properties"]["type"]["enum"], json!([tag]));
        assert_eq!(variant["additionalProperties"], false);
    }
    assert_eq!(variants.len(), 4);
    let vless = &variants[2];
    assert_eq!(
        vless["required"],
        json!(["options", "stream", "flow", "uuid", "tls", "type"])
    );
    assert_eq!(
        vless["properties"]["tls"]["$ref"],
        "#/components/schemas/ClientTls"
    );
    // The nullable reference may describe itself; the target stays the same.
    let websocket = vless["properties"]["websocket"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(websocket.len(), 2);
    assert_eq!(
        websocket[0]["$ref"],
        "#/components/schemas/ListenerWebSocket"
    );
    assert_eq!(websocket[1], json!({"type": "null"}));
}
