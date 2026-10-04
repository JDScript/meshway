use meshway_configuration::*;

#[test]
fn listener_deletion_requires_explicit_null_and_has_a_closed_required_schema() {
    let id = "12345678-1234-0234-0234-123456789abc";
    assert!(
        serde_json::from_str::<ConfigurationOperation>(&format!(
            r#"{{"type":"setListener","nodeId":"{id}","key":"edge"}}"#
        ))
        .is_err()
    );
    assert!(matches!(
        serde_json::from_str::<ConfigurationOperation>(&format!(
            r#"{{"type":"setListener","nodeId":"{id}","key":"edge","listener":null}}"#
        ))
        .unwrap(),
        ConfigurationOperation::SetListener { listener: None, .. }
    ));
    let schema =
        serde_json::to_value(<ConfigurationOperation as utoipa::PartialSchema>::schema()).unwrap();
    let variant = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"]["type"]["enum"][0] == "setListener")
        .unwrap();
    assert_eq!(variant["additionalProperties"], false);
    assert!(
        variant["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "listener")
    );
}

#[test]
fn edits_preserve_unmodified_comments_and_reject_duplicate_fields() {
    let source = "// header\n{\"dns\": {\"domain\": \"example.test\", /* retained */ \"magicDNS\":true},\"groups\": {\"group:old\":[], /* keep */ \"group:keep\":[]}} // tail";
    let before = parse_jsonc(source).unwrap();
    let mut groups = before.groups.clone();
    groups.remove("group:old");
    groups.insert("group:new".into(), vec![]);
    let (edited, result) =
        edit_source(source, &[ConfigurationOperation::ReplaceGroups { groups }]).unwrap();
    assert!(edited.starts_with("// header"));
    assert!(edited.ends_with("// tail"));
    assert!(edited.contains("/* retained */"));
    assert!(edited.contains("/* keep */"));
    assert!(result.groups.contains_key("group:new"));
    assert!(!result.groups.contains_key("group:old"));
    assert_eq!(edit_source(&edited, &[]).unwrap().0, edited);
    assert!(
        parse_jsonc("{\"dns\":{\"domain\":\"example.test\",\"domain\":\"other.test\"}}").is_err()
    );
}

#[test]
fn final_diff_points_at_the_deepest_changed_member_including_array_elements() {
    let config = new_network(
        AddressPools {
            ipv4: vec![],
            ipv6: vec![],
        },
        "example.test".into(),
    )
    .unwrap();
    assert!(!config.access.is_empty());
    let mut cleared = config.clone();
    cleared.access.clear();
    let diff = configuration_changes(&config, &cleared).unwrap();
    assert_eq!(diff.len(), config.access.len());
    assert!(
        diff.iter()
            .enumerate()
            .all(|(i, change)| change.path == format!("/access/{i}")
                && change.kind == ChangeKind::Removed)
    );
    let mut remarked = config.clone();
    remarked.access[0].remark = "annotated".into();
    let diff = configuration_changes(&config, &remarked).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "/access/0/remark");
    assert_eq!(diff[0].kind, ChangeKind::Changed);
    let mut appended = config.clone();
    appended.access.push(config.access[0].clone());
    let diff = configuration_changes(&config, &appended).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, format!("/access/{}", config.access.len()));
    assert_eq!(diff[0].kind, ChangeKind::Added);
    assert!(configuration_changes(&config, &config).unwrap().is_empty());
}

#[test]
fn removing_all_properties_and_adding_with_comments_is_valid() {
    for source in [
        "{\"dns\":{\"domain\":\"example.test\"},\"groups\":{\"group:a\":[],\"group:b\":[]}}",
        "{\"dns\":{\"domain\":\"example.test\"},\"groups\":{\"group:a\":[],// comma , ignored\n\"group:b\":[],}}",
    ] {
        let mut groups = std::collections::BTreeMap::new();
        groups.insert("group:c".into(), vec![]);
        let (edited, result) =
            edit_source(source, &[ConfigurationOperation::ReplaceGroups { groups }]).unwrap();
        assert_eq!(parse_jsonc(&edited).unwrap(), result);
        assert_eq!(result.groups.len(), 1);
    }
}
