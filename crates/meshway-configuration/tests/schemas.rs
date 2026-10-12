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
        11
    );
    assert_eq!(
        schemas["NetworkConfiguration"]["properties"]["ruleSetProviders"]["additionalProperties"]["$ref"],
        "#/components/schemas/NullableRuleSetProvider"
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

#[test]
fn mixed_preparation_kind_only_uses_object_form_for_material() {
    for kind in [
        "addressPoolV4",
        "addressPoolV6",
        "addressV4",
        "addressV6",
        "identityProof",
        "ruleSetContent",
    ] {
        let value: PreparationKind = serde_json::from_value(serde_json::json!(kind)).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), kind);
        assert!(
            serde_json::from_value::<PreparationKind>(serde_json::json!({(kind):null})).is_err()
        );
    }
    let material = serde_json::json!({"material":"certificate"});
    let value: PreparationKind = serde_json::from_value(material.clone()).unwrap();
    assert_eq!(serde_json::to_value(value).unwrap(), material);
    for invalid in [
        serde_json::json!("material"),
        serde_json::json!({"material":"certificate","extra":true}),
        serde_json::json!(["certificate"]),
    ] {
        assert!(serde_json::from_value::<PreparationKind>(invalid).is_err());
    }
    assert!(
        serde_json::from_str::<PreparationKind>(
            r#"{"material":"certificate","material":"privateKey"}"#
        )
        .is_err()
    );
}
