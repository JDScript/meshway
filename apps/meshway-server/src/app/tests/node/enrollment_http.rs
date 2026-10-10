#[test]
fn node_and_management_configuration_openapi_components_are_distinct() {
    let spec = serde_json::to_value(crate::http::openapi()).unwrap();
    let schemas = &spec["components"]["schemas"];
    assert!(schemas["ConfigurationRequest"]["oneOf"].is_array());
    assert!(schemas["SyncRequest"]["properties"]["netmap"].is_object());
    assert!(schemas["ConfigurationResponse"]["properties"]["configuration"].is_object());
    assert!(schemas["SyncResponse"]["properties"]["lease"].is_object());
    assert_eq!(
        spec["paths"]["/agent-api/v1/sync"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/SyncRequest"
    );
}
