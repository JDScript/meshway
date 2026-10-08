use super::*;

#[test]
fn declaration_preview_defers_material_only_and_cannot_normalize_as_execution_input() {
    let mut input = with_nodes();
    input.nodes.get_mut(&id(1)).unwrap().listeners.insert(
        "vless".into(),
        serde_json::from_str(
            r#"{"type":"vless","listenPort":443,"tls":{"serverName":"node.example.test"}}"#,
        )
        .unwrap(),
    );
    let plan = plan_configuration(input.clone(), &context()).unwrap();
    assert_eq!(plan.requirements.len(), 4);
    assert!(!plan.view.nodes[&id(1)].listeners["vless"].credential_present);
    assert_eq!(
        plan.input.nodes[&id(1)].listeners["vless"],
        input.nodes[&id(1)].listeners["vless"]
    );
    assert!(matches!(
        normalize_with_dependencies(plan.input, &context()),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    let mut no_identity = context();
    no_identity.approved_identities.clear();
    assert!(matches!(
        plan_configuration(input.clone(), &no_identity),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["group:missing".into()],
        dst: vec!["*".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    assert!(matches!(
        plan_configuration(input, &context()),
        Err(ConfigurationError::Invalid(_))
    ));
}

#[test]
fn missing_material_is_a_requirement_and_debug_never_exposes_it() {
    let mut c = with_nodes();
    let listener: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"stream":"websocket","tls":{"serverName":"Edge.Example"}}"#).unwrap();
    c.nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .insert("web".into(), listener);
    let ConfigurationError::PreparationRequired(requirements) =
        normalize(c, &context()).unwrap_err()
    else {
        panic!()
    };
    for kind in [
        MaterialRequirement::Uuid,
        MaterialRequirement::Certificate,
        MaterialRequirement::PrivateKey,
        MaterialRequirement::CertificateValidation,
    ] {
        assert!(
            requirements
                .iter()
                .any(|r| r.kind == PreparationKind::Material(kind))
        );
    }
    let source = "// super-secret\n{\"addressPools\":{\"ipv4\":[],\"ipv6\":[]},\"dns\":{\"domain\":\"mesh.example\"}}".to_owned();
    let (configuration, _) =
        normalize(parse_jsonc(&source).unwrap(), &ValidationContext::default()).unwrap();
    let document = ConfigurationDocument::new(source, configuration);
    assert!(!format!("{document:?}").contains("super-secret"));
}

#[test]
fn listener_material_retention_requires_the_same_resource_and_exact_attestation() {
    let mut c = with_nodes();
    let mut declaration: Listener = serde_json::from_str(r#"{"type":"vless","listenPort":443,"uuid":"11111111-1111-4111-8111-111111111111","tls":{"serverName":"edge.example","certificate":"certificate fixture","privateKey":"key fixture"}}"#).unwrap();
    declaration.normalize().unwrap();
    c.nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .insert("edge".into(), declaration.clone());
    assert!(matches!(
        normalize(c.clone(), &context()),
        Err(ConfigurationError::PreparationRequired(_))
    ));
    let mut ctx = context();
    ctx.verified_listeners
        .insert((id(1), "edge".into()), declaration.clone());
    let (old, _) = normalize(c, &ctx).unwrap();
    let mut edit = NetworkConfigurationInput::from(&old);
    let Listener::Vless(v) = edit
        .nodes
        .get_mut(&id(1))
        .unwrap()
        .listeners
        .get_mut("edge")
        .unwrap()
    else {
        panic!()
    };
    v.uuid = None;
    v.tls.certificate = None;
    v.tls.private_key = None;
    ctx.previous = Some(&old);
    assert_eq!(
        normalize(edit.clone(), &ctx).unwrap().0.nodes[&id(1)].listeners,
        old.nodes[&id(1)].listeners
    );
    let mut deleted = old.clone();
    deleted.nodes.get_mut(&id(1)).unwrap().listeners.clear();
    ctx.previous = Some(&deleted);
    let ConfigurationError::PreparationRequired(requirements) = normalize(edit, &ctx).unwrap_err()
    else {
        panic!()
    };
    assert!(
        requirements
            .iter()
            .any(|r| r.kind == PreparationKind::Material(MaterialRequirement::Uuid))
    );
}

#[test]
fn remote_rule_sources_are_typed_https_declarations() {
    for url in [
        "http://example.com/rules",
        "https://user:password@example.com/rules",
        "https://:443/rules",
        "https://example.com:bad/rules",
    ] {
        let mut c = input();
        c.rule_sets.insert(
            "external".into(),
            RuleSet::Remote {
                url: url.into(),
                format: RuleSetFormat::SingBoxSource,
                update_interval: "24h".parse().unwrap(),
            },
        );
        invalid(normalize(c, &context()), "HTTPS");
    }
}
