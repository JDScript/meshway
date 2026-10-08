use super::*;

#[test]
fn diagnostics_carry_closed_codes_parameters_and_precise_pointers() {
    // A node-level exit authorization problem points at the rule and names the node.
    let found = issues(normalize_with_dependencies(
        egress_input(&["8.8.8.0/25"], "8.8.8.0/24"),
        &context(),
    ));
    let outside = found
        .iter()
        .find(|i| i.code == IssueCode::ExitOutsideAccess)
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(outside.path, "/routing/egress/profiles/work/rules/0");
    assert_eq!(outside.params["node"], id(1).to_string());
    assert_eq!(outside.params["destination"], "8.8.8.0/24");
    assert_eq!(outside.message, IssueCode::ExitOutsideAccess.message());
    // Selector problems point at the element, not the array.
    let mut c = with_nodes();
    c.access.push(AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", id(1)), "group:nobody".into()],
        dst: vec!["mallory@example.com".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    let found = issues(normalize_with_dependencies(c, &context()));
    let group = found
        .iter()
        .find(|i| i.code == IssueCode::UnknownGroup)
        .unwrap();
    assert_eq!(group.path, "/access/1/src/1");
    assert_eq!(group.params["name"], "group:nobody");
    let user = found
        .iter()
        .find(|i| i.code == IssueCode::UnknownUser)
        .unwrap();
    assert_eq!(user.path, "/access/1/dst/0");
    assert_eq!(user.params["user"], "mallory@example.com");
    // Transport declaration problems carry the declaration code.
    let mut c = with_nodes();
    c.nodes.get_mut(&id(1)).unwrap().listeners.insert(
        "vless".into(),
        serde_json::from_str(
            r#"{"type":"vless","listenPort":443,"flow":"xtls","tls":{"serverName":"node.example.test"}}"#,
        )
        .unwrap(),
    );
    let found = issues(normalize_with_dependencies(c, &context()));
    let transport = found
        .iter()
        .find(|i| i.code == IssueCode::InvalidTransport)
        .unwrap();
    assert_eq!(transport.path, format!("/nodes/{}/listeners/vless", id(1)));
    assert_eq!(transport.params["reason"], "vless_flow");
    // Serialized shape: code is snake_case, params always present.
    let json = serde_json::to_value(transport).unwrap();
    assert_eq!(json["code"], "invalid_transport");
    assert!(json["params"].is_object());
}

#[test]
fn source_parse_errors_report_position_without_echoing_text() {
    let syntax = parse_jsonc("{\"dns\": {\"domain\": \"x.test\",}} trailing").unwrap_err();
    let ConfigurationError::Invalid(found) = syntax else {
        panic!("{syntax:?}")
    };
    assert_eq!(found[0].code, IssueCode::InvalidJsonc);
    assert_eq!(found[0].params["line"], "1");
    assert!(found[0].params.contains_key("column"));
    let shape =
        parse_jsonc("{\n  \"dns\": {\"domain\": \"x.test\"},\n  \"groups\": \"s3cret-value\"\n}")
            .unwrap_err();
    let ConfigurationError::Invalid(found) = shape else {
        panic!("{shape:?}")
    };
    assert_eq!(found[0].code, IssueCode::InvalidShape);
    assert_eq!(found[0].params["line"], "3");
    assert!(!format!("{found:?}").contains("s3cret"));
}
