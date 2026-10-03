use meshway_protocol::history::ActivityAction;
use utoipa::PartialSchema;

#[test]
fn activity_schema_values_are_exactly_accepted_and_serialized_wire_actions() {
    let schema = serde_json::to_value(ActivityAction::schema()).unwrap();
    let values = schema["enum"].as_array().unwrap();
    assert!(values.contains(&serde_json::json!("session.login")));
    assert!(values.contains(&serde_json::json!("membership.role")));
    assert!(!values.contains(&serde_json::json!("SessionLogin")));
    for value in values {
        let action: ActivityAction = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(action).unwrap(), *value);
    }
    assert!(serde_json::from_str::<ActivityAction>(r#""SessionLogin""#).is_err());
}
