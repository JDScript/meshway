use meshway_model::{OuterSocket, TransportProtocol, VlessStream};
use meshway_transport::spec::{
    Port, PositiveDuration, TlsVersion, WebSocketHeaderName, WebSocketHeaderValue,
    WebSocketHeaders, WebSocketPath,
};
use serde_json::{Value, json};
use utoipa::{OpenApi, PartialSchema, ToSchema};

fn schema<T: PartialSchema>() -> Value {
    serde_json::to_value(T::schema()).unwrap()
}

#[test]
fn schemas_keep_scalar_wire_types_and_closed_protocol_names() {
    let port = schema::<Port>();
    assert_eq!(port["type"], "integer");
    assert_eq!(port["minimum"], 1);
    assert_eq!(port["maximum"], 65535);

    for (schema, names) in [
        (
            schema::<TransportProtocol>(),
            json!(["native_udp", "native_tcp", "vless", "trojan"]),
        ),
        (schema::<OuterSocket>(), json!(["udp", "tcp"])),
        (schema::<VlessStream>(), json!(["tcp", "websocket"])),
        (schema::<TlsVersion>(), json!(["1.2", "1.3"])),
    ] {
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["enum"], names);
    }
    for schema in [
        schema::<PositiveDuration>(),
        schema::<WebSocketPath>(),
        schema::<WebSocketHeaderName>(),
        schema::<WebSocketHeaderValue>(),
    ] {
        assert_eq!(schema["type"], "string");
        let pattern = schema["pattern"].as_str().unwrap();
        assert!(pattern.starts_with('^'));
        // `$` would accept a final newline in ECMAScript. All patterns instead
        // require the actual end of input, including for duration unit suffixes.
        assert!(pattern.ends_with(r"(?![\s\S])"));
    }
}

#[test]
fn duration_schema_encodes_each_units_overflow_boundary() {
    let schema = schema::<PositiveDuration>();
    let pattern = schema["pattern"].as_str().unwrap();
    for (unit, multiplier) in [("ms", 1), ("s", 1000), ("m", 60000), ("h", 3600000)] {
        assert!(pattern.contains(&format!("{}){unit}", u64::MAX / multiplier)));
    }
}

#[test]
fn headers_schema_constrains_keys_and_values_instead_of_an_untyped_object() {
    let headers = schema::<WebSocketHeaders>();
    assert_eq!(headers["type"], "object");
    assert_eq!(headers["propertyNames"], schema::<WebSocketHeaderName>());
    assert_eq!(
        headers["additionalProperties"],
        schema::<WebSocketHeaderValue>()
    );
}

#[test]
fn public_types_register_in_openapi_without_private_wire_helpers() {
    #[derive(OpenApi)]
    #[openapi(components(schemas(
        Port,
        PositiveDuration,
        TransportProtocol,
        OuterSocket,
        VlessStream,
        TlsVersion,
        WebSocketPath,
        WebSocketHeaderName,
        WebSocketHeaderValue,
        WebSocketHeaders
    )))]
    struct Document;

    let document = serde_json::to_value(Document::openapi()).unwrap();
    let components = document["components"]["schemas"].as_object().unwrap();
    assert_eq!(components.len(), 10);
    assert_eq!(components["PositiveDuration"], schema::<PositiveDuration>());
    assert_eq!(components["WebSocketHeaders"], schema::<WebSocketHeaders>());
}

#[test]
fn duration_fields_use_the_validated_schema_instead_of_a_builtin_duration() {
    #[derive(ToSchema)]
    #[allow(dead_code)]
    struct Parameters {
        timeout: PositiveDuration,
    }

    assert_eq!(
        schema::<Parameters>()["properties"]["timeout"]["$ref"],
        "#/components/schemas/PositiveDuration"
    );
}
