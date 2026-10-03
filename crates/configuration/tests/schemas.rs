use meshway_configuration::*;
use utoipa::{OpenApi, PartialSchema};

#[derive(OpenApi)]
#[openapi(components(schemas(NetworkConfiguration, NetworkConfigurationInput)))]
struct Api;

#[test]
fn schemas_register_complete_node_protocols_and_exact_nullable_boundaries() {
    let api = serde_json::to_value(Api::openapi()).unwrap();
    let schemas = &api["components"]["schemas"];
    for name in [
        "NetworkConfiguration",
        "NodeSettings",
        "NodeIdentity",
        "DnsConfiguration",
        "RoutingConfiguration",
        "PreparedListener",
        "Listener",
        "DialerSet",
    ] {
        assert!(schemas.get(name).is_some(), "missing {name}");
    }
    assert_eq!(
        schemas["NetworkConfiguration"]["additionalProperties"],
        false
    );
    assert_eq!(
        schemas["NetworkConfiguration"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        schemas["NodeSettings"]["properties"]["listeners"]["additionalProperties"]["$ref"],
        "#/components/schemas/PreparedListener"
    );
    let addresses = serde_json::to_value(NodeAddresses::schema()).unwrap();
    assert!(
        addresses["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "ipv4")
    );
    assert!(
        addresses["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "ipv6")
    );
    assert!(serde_json::from_str::<NodeAddresses>(r#"{"ipv4":null,"ipv6":null}"#).is_ok());
    assert!(serde_json::from_str::<NodeAddresses>(r#"{"ipv4":null}"#).is_err());
}

#[test]
fn tagged_objects_reject_sequence_forms_and_unit_branch_extra_fields() {
    for text in [
        r#"["direct"]"#,
        r#"{"type":"direct","nodes":[]}"#,
        r#"{"type":"direct","type":"unmanaged"}"#,
    ] {
        assert!(serde_json::from_str::<RouteAction>(text).is_err(), "{text}");
    }
    for text in [
        r#"{"type":"inline","ipCIDR":[],"url":"https://example.com"}"#,
        r#"["inline",[],[]]"#,
    ] {
        assert!(serde_json::from_str::<RuleSet>(text).is_err(), "{text}");
    }
}
