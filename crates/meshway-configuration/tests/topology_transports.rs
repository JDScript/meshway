use meshway_configuration::{Input, TopologyRule};
use meshway_model::TransportProtocol;
use serde_json::json;

#[test]
fn topology_transports_distinguish_omission_empty_and_supported_wire_names() {
    let base = json!({"action": "allow", "a": "*", "b": "*"});
    let omitted: TopologyRule = serde_json::from_value(base.clone()).unwrap();
    assert_eq!(omitted.transports, Input::Omitted);
    assert!(
        serde_json::to_value(omitted)
            .unwrap()
            .get("transports")
            .is_none()
    );
    for (values, expected) in [
        (json!([]), vec![]),
        (
            json!(["native_udp", "native_tcp", "vless", "trojan"]),
            vec![
                TransportProtocol::NativeUdp,
                TransportProtocol::NativeTcp,
                TransportProtocol::Vless,
                TransportProtocol::Trojan,
            ],
        ),
    ] {
        let mut value = base.clone();
        value["transports"] = values.clone();
        let rule: TopologyRule = serde_json::from_value(value).unwrap();
        assert_eq!(rule.transports, Input::Value(expected));
        assert_eq!(serde_json::to_value(rule).unwrap()["transports"], values);
    }
}

#[test]
fn topology_transports_reject_null_unknown_names_and_wrong_shapes() {
    for transports in [
        json!(null),
        json!(["udp"]),
        json!(["nativeUdp"]),
        json!(["masque"]),
        json!("native_udp"),
        json!([1]),
        json!([null]),
    ] {
        assert!(
            serde_json::from_value::<TopologyRule>(json!({
                "action": "allow", "a": "*", "b": "*", "transports": transports,
            }))
            .is_err()
        );
    }
}
