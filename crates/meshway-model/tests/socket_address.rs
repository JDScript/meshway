use meshway_model::SocketAddress;
use serde_json::json;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use utoipa::PartialSchema;

#[test]
fn numeric_aliases_are_accepted_and_serialized_with_std_display() {
    for (input, canonical) in [
        ("127.0.0.1:00000080", "127.0.0.1:80"),
        ("192.0.2.1:65535", "192.0.2.1:65535"),
        (
            "[2001:0DB8:0000:0000:0000:0000:0000:0001]:443",
            "[2001:db8::1]:443",
        ),
        ("[::FFFF:192.0.2.1]:53", "[::ffff:192.0.2.1]:53"),
        ("[::1%0]:53", "[::1]:53"),
        ("[::1%000]:00053", "[::1]:53"),
    ] {
        let address: SocketAddress = serde_json::from_value(json!(input)).unwrap();
        assert_eq!(address.to_string(), canonical);
        assert_eq!(address.get(), input.parse::<SocketAddr>().unwrap());
        assert_eq!(serde_json::to_value(address).unwrap(), json!(canonical));
        assert_eq!(SocketAddress::try_from(address.get()).unwrap(), address);
    }
    // No text-length limit narrower than std's numeric parser is imposed.
    let long_port = format!("127.0.0.1:{}80", "0".repeat(256));
    assert_eq!(
        long_port.parse::<SocketAddress>().unwrap().to_string(),
        "127.0.0.1:80"
    );
}

#[test]
fn nonnumeric_zero_port_scoped_and_nonstring_values_are_rejected() {
    for invalid in [
        "example.test:80",
        "127.0.0.1:0",
        "[::1]:0",
        "[fe80::1%2]:53",
        "[fe80::1%eth0]:53",
        "127.0.0.1:+80",
        "127.0.0.1:65536",
        "127.0.0.1",
        "::1:80",
        "127.000.0.1:80",
        " 127.0.0.1:80",
        "127.0.0.1:80 ",
    ] {
        assert!(
            serde_json::from_value::<SocketAddress>(json!(invalid)).is_err(),
            "{invalid}"
        );
    }
    for invalid in [
        json!(null),
        json!(53),
        json!(["127.0.0.1", 80]),
        json!({"address":"127.0.0.1:80"}),
    ] {
        assert!(serde_json::from_value::<SocketAddress>(invalid).is_err());
    }
    for (port, flowinfo, scope) in [(0, 0, 0), (53, 1, 0), (53, 0, 1)] {
        let address = SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::LOCALHOST,
            port,
            flowinfo,
            scope,
        ));
        assert!(SocketAddress::new(address).is_err());
        assert!(SocketAddress::try_from(address).is_err());
    }
}

#[test]
fn schema_uses_the_numeric_socket_format() {
    let schema = serde_json::to_value(SocketAddress::schema()).unwrap();
    assert_eq!(schema["type"], "string");
    assert_eq!(schema["format"], "meshway-socket-address");
    assert!(schema.get("maxLength").is_none());
}
