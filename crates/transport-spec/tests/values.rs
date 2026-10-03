use meshway_transport_spec::{
    OuterSocket, Port, PositiveDuration, TlsVersion, TlsVersionRange, TransportProtocol,
    ValidationError, VlessStream, WebSocketHeaderName, WebSocketHeaderValue, WebSocketHeaders,
    WebSocketPath, validate_websocket_alpn,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[test]
fn protocol_keys_and_socket_layers_are_distinct() {
    assert!(serde_json::from_value::<TransportProtocol>(json!({"vless": null})).is_err());
    assert!(serde_json::from_value::<OuterSocket>(json!({"tcp": null})).is_err());
    assert!(serde_json::from_value::<VlessStream>(json!({"tcp": null})).is_err());
    assert!(serde_json::from_value::<TlsVersion>(json!({"1.2": null})).is_err());
    for (key, protocol, outer) in [
        ("native_udp", TransportProtocol::NativeUdp, OuterSocket::Udp),
        ("native_tcp", TransportProtocol::NativeTcp, OuterSocket::Tcp),
        ("vless", TransportProtocol::Vless, OuterSocket::Tcp),
        ("trojan", TransportProtocol::Trojan, OuterSocket::Tcp),
    ] {
        assert_eq!(
            serde_json::from_value::<TransportProtocol>(json!(key)).unwrap(),
            protocol
        );
        assert_eq!(serde_json::to_value(protocol).unwrap(), key);
        assert_eq!(protocol.as_str(), key);
        assert_eq!(protocol.outer_socket(), outer);
    }
    for key in ["nativeUdp", "udp", "VLESS", "quic", "reality", ""] {
        assert!(
            serde_json::from_value::<TransportProtocol>(json!(key)).is_err(),
            "{key}"
        );
    }
    for (key, stream) in [
        ("tcp", VlessStream::Tcp),
        ("websocket", VlessStream::WebSocket),
    ] {
        assert_eq!(
            serde_json::from_value::<VlessStream>(json!(key)).unwrap(),
            stream
        );
        assert_eq!(serde_json::to_value(stream).unwrap(), key);
        assert_eq!(stream.outer_socket(), OuterSocket::Tcp);
    }
    for key in ["ws", "auto", "udp", "httpUpgrade", "h2"] {
        assert!(serde_json::from_value::<VlessStream>(json!(key)).is_err());
    }
}

#[test]
fn ports_validate_without_numeric_coercion() {
    for port in [1, 443, 65535] {
        let value = Port::new(port).unwrap();
        assert_eq!(value.get(), port);
        assert_eq!(serde_json::from_value::<Port>(json!(port)).unwrap(), value);
        assert_eq!(serde_json::to_value(value).unwrap(), json!(port));
    }
    assert_eq!(Port::new(0), Err(ValidationError::Port));
    for invalid in [
        json!(0),
        json!(65536),
        json!(-1),
        json!(1.5),
        json!("443"),
        json!(null),
        json!({}),
        json!(["1.2", "1.3"]),
    ] {
        assert!(
            serde_json::from_value::<Port>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn durations_preserve_units_and_check_the_total_millisecond_boundary() {
    for (source, milliseconds) in [("1ms", 1), ("75s", 75000), ("5m", 300000), ("2h", 7200000)] {
        let value = source.parse::<PositiveDuration>().unwrap();
        assert_eq!(value.as_str(), source);
        assert_eq!(value.as_millis(), milliseconds);
        assert_eq!(
            value.as_std(),
            std::time::Duration::from_millis(milliseconds)
        );
        assert_eq!(serde_json::to_value(&value).unwrap(), source);
        assert_eq!(
            serde_json::from_value::<PositiveDuration>(json!(source)).unwrap(),
            value
        );
    }
    for (unit, multiplier) in [("ms", 1), ("s", 1000), ("m", 60000), ("h", 3600000)] {
        let limit = u64::MAX / multiplier;
        let source = format!("{limit}{unit}");
        let duration = source.parse::<PositiveDuration>().unwrap();
        assert_eq!(duration.as_millis(), limit * multiplier);
        let overflow = format!("{}{unit}", u128::from(limit) + 1);
        assert_eq!(
            overflow.parse::<PositiveDuration>(),
            Err(ValidationError::DurationRange)
        );
        assert!(serde_json::from_value::<PositiveDuration>(json!(overflow)).is_err());
    }
    for invalid in [
        "", "0ms", "0h", "01s", "+1s", "-1s", "1.5s", "1e3ms", "1d", "1S", "1", "s", "1 s", " 1s",
        "1s\n", "１s", "1h30m",
    ] {
        assert!(invalid.parse::<PositiveDuration>().is_err(), "{invalid:?}");
        assert!(serde_json::from_value::<PositiveDuration>(json!(invalid)).is_err());
    }
    for invalid in [json!(1000), json!(null), json!({"seconds": 1})] {
        assert!(serde_json::from_value::<PositiveDuration>(invalid).is_err());
    }
}

#[test]
fn websocket_paths_are_exact_and_do_not_admit_request_line_injection() {
    for path in ["/", "/meshway", "/a/b", "/网格", "/%3F", "/a%20b"] {
        let value = path.parse::<WebSocketPath>().unwrap();
        assert_eq!(value.as_str(), path);
        assert_eq!(serde_json::to_value(value).unwrap(), path);
    }
    for path in [
        "",
        "meshway",
        "https://example.test/a",
        "/?q=x",
        "/#fragment",
        "/a b",
        "/a\tb",
        "/a\r\nHost:x",
        "/\0",
        "/\u{7f}",
        "/\u{85}",
        "/\u{a0}",
        "/\u{2003}",
        "/\u{2028}",
    ] {
        assert!(path.parse::<WebSocketPath>().is_err(), "{path:?}");
        assert!(serde_json::from_value::<WebSocketPath>(json!(path)).is_err());
    }
}

#[test]
fn websocket_headers_preserve_casing_and_prevent_handshake_overrides() {
    let source =
        json!({"X-Trace": "one", "x-Other": "two\tthree", "X-Empty": "", "X-Text": "网格"});
    let headers: WebSocketHeaders = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(headers.len(), 4);
    assert!(!headers.is_empty());
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "X-Trace" && value == "one")
    );
    assert_eq!(serde_json::to_value(&headers).unwrap(), source);
    assert!(WebSocketHeaders::default().is_empty());
    for name in [
        "Host",
        "hOsT",
        "Connection",
        "CONNECTION",
        "Upgrade",
        "uPgRaDe",
        "Sec-WebSocket-Key",
        "sEc-WeBsOcKeT-aCcEpT",
        "Sec-WebSocket-Protocol",
        "Sec-WebSocket-Version",
        "Sec-WebSocket-Extensions",
        "Sec-WebSocket-",
        "Sec-WebSocket-Future",
    ] {
        assert_eq!(
            name.parse::<WebSocketHeaderName>(),
            Err(ValidationError::ReservedHeader),
            "{name}"
        );
        assert!(serde_json::from_value::<WebSocketHeaders>(json!({name: "value"})).is_err());
    }
    for name in [
        "X-Host",
        "Host-Extra",
        "X-Sec-WebSocket-Key",
        "!#$%&'*+-.^_`|~09AZaz",
    ] {
        assert_eq!(name.parse::<WebSocketHeaderName>().unwrap().as_str(), name);
    }
    for name in [
        "",
        "x:y",
        "bad name",
        "bad\tname",
        "x\r\nHost",
        "é",
        "@name",
        "[name]",
    ] {
        assert_eq!(
            name.parse::<WebSocketHeaderName>(),
            Err(ValidationError::HeaderName),
            "{name:?}"
        );
    }
    for value in [
        "x\r\nHost: evil",
        "\n",
        "\r",
        "\0",
        "\u{b}",
        "\u{1f}",
        "\u{7f}",
        "\u{85}",
    ] {
        assert_eq!(
            value.parse::<WebSocketHeaderValue>(),
            Err(ValidationError::HeaderValue)
        );
        assert!(serde_json::from_value::<WebSocketHeaders>(json!({"X-Test": value})).is_err());
    }
}

#[test]
fn duplicate_and_non_string_headers_are_not_silently_overwritten_or_coerced() {
    for source in [
        r#"{"X-Test":"a","X-Test":"b"}"#,
        r#"{"X-Test":"a","x-test":"b"}"#,
        r#"{"X-Test":null}"#,
        r#"{"X-Test":["a","b"]}"#,
        r#"{"X-Test":1}"#,
        r#"[]"#,
        r#"null"#,
    ] {
        assert!(
            serde_json::from_str::<WebSocketHeaders>(source).is_err(),
            "{source}"
        );
    }
    assert_eq!(
        WebSocketHeaders::try_from_iter([
            ("X-Test".into(), "a".into()),
            ("x-test".into(), "b".into()),
        ]),
        Err(ValidationError::DuplicateHeader)
    );
}

#[test]
fn tls_range_is_validated_on_construction_and_strict_deserialization() {
    for (min, max) in [
        (TlsVersion::Tls12, TlsVersion::Tls12),
        (TlsVersion::Tls12, TlsVersion::Tls13),
        (TlsVersion::Tls13, TlsVersion::Tls13),
    ] {
        let value = TlsVersionRange::new(min, max).unwrap();
        let source = json!({"minVersion": min.as_str(), "maxVersion": max.as_str()});
        assert_eq!(value.min_version(), min);
        assert_eq!(value.max_version(), max);
        assert_eq!(serde_json::to_value(value).unwrap(), source);
        assert_eq!(
            serde_json::from_value::<TlsVersionRange>(source).unwrap(),
            value
        );
    }
    assert_eq!(
        TlsVersionRange::new(TlsVersion::Tls13, TlsVersion::Tls12),
        Err(ValidationError::TlsVersionOrder)
    );
    for source in [
        json!({"minVersion": "1.3", "maxVersion": "1.2"}),
        json!({"minVersion": "1.1", "maxVersion": "1.3"}),
        json!({"minVersion": "1.2", "maxVersion": "1.4"}),
        json!({"minVersion": 1.2, "maxVersion": 1.3}),
        json!({"minVersion": "1.2"}),
        json!({"minVersion": "1.2", "maxVersion": "1.3", "insecure": true}),
        json!({"min_version": "1.2", "max_version": "1.3"}),
        json!({}),
        json!(["1.2", "1.3"]),
    ] {
        assert!(
            serde_json::from_value::<TlsVersionRange>(source.clone()).is_err(),
            "{source}"
        );
    }
    assert!(
        serde_json::from_str::<TlsVersionRange>(
            r#"{"minVersion":"1.2","minVersion":"1.3","maxVersion":"1.3"}"#
        )
        .is_err()
    );
}

#[test]
fn websocket_alpn_does_not_allow_an_incompatible_advertised_alternative() {
    for alpn in [vec![], vec!["http/1.1"], vec!["http/1.1", "http/1.1"]] {
        assert!(
            validate_websocket_alpn(&alpn.into_iter().map(String::from).collect::<Vec<_>>())
                .is_ok()
        );
    }
    for alpn in [
        vec!["h2"],
        vec!["http/1.1", "h2"],
        vec!["h2", "http/1.1"],
        vec!["HTTP/1.1"],
        vec!["http/1.0"],
        vec![""],
    ] {
        assert_eq!(
            validate_websocket_alpn(&alpn.into_iter().map(String::from).collect::<Vec<_>>()),
            Err(ValidationError::WebSocketAlpn)
        );
    }
}

#[test]
fn values_compose_without_weakening_nested_validation() {
    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Parameters {
        listen_port: Port,
        connect_timeout: PositiveDuration,
        path: WebSocketPath,
        response_headers: WebSocketHeaders,
        versions: TlsVersionRange,
    }
    let source = json!({
        "listenPort": 443,
        "connectTimeout": "5s",
        "path": "/meshway",
        "responseHeaders": {"X-Trace": "ok"},
        "versions": {"minVersion": "1.2", "maxVersion": "1.3"}
    });
    let value: Parameters = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(serde_json::to_value(value).unwrap(), source);
    for (field, invalid) in [
        ("listenPort", json!(0)),
        ("connectTimeout", json!("0s")),
        ("path", json!("/\r\n")),
        ("responseHeaders", json!({"Host": "evil"})),
        ("versions", Value::Null),
        ("earlyData", json!(true)),
    ] {
        let mut candidate = source.clone();
        candidate[field] = invalid;
        assert!(
            serde_json::from_value::<Parameters>(candidate).is_err(),
            "{field}"
        );
    }
}
