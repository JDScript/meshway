use super::*;

#[tokio::test]
async fn nonsecret_defaults_edit_keeps_and_explicitly_clears_websocket_headers() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "headers@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "headers")
        .await
        .network
        .id;
    let initial = source(&c, &s, n, owner.token.expose()).await;
    let mut input = initial.configuration.clone();
    input.defaults.dialers.vless.as_mut().unwrap().websocket =
        Some(serde_json::from_str(r#"{"headers":{"Authorization":"private value"}}"#).unwrap());
    c.post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Document {
            expected_revision: initial.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    for (websocket, expected_count) in [(r#"{}"#, 1), (r#"{"headers":{}}"#, 0)] {
        let current = source(&c, &s, n, owner.token.expose()).await;
        let op = serde_json::from_str(&format!(r#"{{"type":"replaceDefaults","defaults":{{"dialers":{{"vless":{{"remark":"edge","websocket":{websocket}}}}}}}}}"#)).unwrap();
        let edit = ConfigurationRequest::Operations {
            expected_revision: current.revision,
            request_id: request(),
            ops: vec![op],
        };
        let preview = c
            .post(net(&s, n, "configuration/validate"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!preview.contains("private value"));
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&edit)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let saved = source(&c, &s, n, owner.token.expose()).await;
        let vless = saved.configuration.defaults.dialers.vless.unwrap();
        assert_eq!(vless.remark, "edge");
        assert_eq!(vless.websocket.unwrap().headers.len(), expected_count);
    }
    s.close().await;
}

#[tokio::test]
async fn advertised_network_name_rules_match_unicode_http_boundary() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let options: AuthOptionsResponse = json(c.get(s.auth("options"))).await;
    assert_eq!(options.network_name_rules, NetworkNameRules::CURRENT);
    let user = register(&c, &s, "names@example.test", "valid password").await;
    reject(
        c.get(format!("{}/api/v1/configuration/schema", s.url)),
        StatusCode::UNAUTHORIZED,
        "invalid_session",
    )
    .await;
    let schema: ConfigurationSchemaResponse = json(
        c.get(format!("{}/api/v1/configuration/schema", s.url))
            .bearer_auth(user.token.expose()),
    )
    .await;
    let schema: serde_json::Value = serde_json::from_str(&schema.schema_json).unwrap();
    fn references(value: &serde_json::Value, root: &serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref") {
                    let reference = reference.as_str().unwrap();
                    assert!(reference.starts_with("#/$defs/"));
                    assert!(
                        root.pointer(&reference[1..]).is_some(),
                        "missing schema {reference}"
                    );
                }
                for child in object.values() {
                    references(child, root);
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    references(child, root);
                }
            }
            _ => {}
        }
    }
    references(&schema, &schema);
    let catalog: TransportCatalogResponse = json(
        c.get(format!("{}/api/v1/transport-catalog", s.url))
            .bearer_auth(user.token.expose()),
    )
    .await;
    assert_eq!(catalog.items.len(), 4);
    for entry in catalog.items {
        assert!(schema["$defs"].get(entry.listener_schema).is_some());
        assert!(schema["$defs"].get(entry.dialer_schema).is_some());
        assert!(entry.required_capability.starts_with("transport:"));
    }
    for (raw, normalized) in [
        (
            format!("\u{2003}{}\u{3000}", "😀".repeat(128)),
            "😀".repeat(128),
        ),
        ("\u{feff}name\u{feff}".into(), "\u{feff}name\u{feff}".into()),
        (
            format!("{}x{}", " ".repeat(511), " ".repeat(512)),
            "x".into(),
        ),
    ] {
        assert_eq!(
            create(&c, &s, user.token.expose(), &raw).await.network.name,
            normalized
        );
    }
    for raw in [
        "😀".repeat(129),
        "\tname".into(),
        "name\u{0085}".into(),
        "a\0b".into(),
        format!("{}x", " ".repeat(1024)),
        "\u{2003}\u{3000}".into(),
    ] {
        reject(
            c.post(format!("{}/api/v1/networks", s.url))
                .bearer_auth(user.token.expose())
                .json(&CreateNetworkRequest {
                    request_id: request(),
                    name: raw,
                }),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    s.close().await;
}

/// M1: MagicDNS and a search domain with no resolver was saved and every
/// Node on resolved withdrew its packet plane. validate and apply refuse it
/// with the same issue for the enrolled Node and store nothing; a split
/// route that answers the domain is saved.
#[tokio::test]
async fn dns_settings_no_node_can_install_are_refused_by_validate_and_apply() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "dns-plan@example.test", "valid password").await;
    let token = owner.token.expose();
    let n = create(&c, &s, token, "dns-plan").await.network.id;
    let node = super::super::live::enroll(&c, &s, n, token, 62)
        .await
        .0
        .node_id;
    let document = |dns: serde_json::Value, revision| {
        let mut input = serde_json::to_value(&revision).unwrap();
        for (key, value) in dns.as_object().unwrap() {
            input["dns"][key] = value.clone();
        }
        input
    };
    let initial = source(&c, &s, n, token).await;
    let request_for = |dns: serde_json::Value| ConfigurationRequest::Document {
        expected_revision: initial.revision,
        request_id: request(),
        source_jsonc: WireSecret::new(
            serde_json::to_string(&document(dns, initial.configuration.clone())).unwrap(),
        ),
    };
    let refused = serde_json::json!({
        "magicDNS": true, "servers": {}, "default": [], "routes": {},
        "searchDomains": ["churn1.m1.test"]
    });
    for endpoint in ["configuration/validate", "configuration"] {
        let response = c
            .post(net(&s, n, endpoint))
            .bearer_auth(token)
            .json(&request_for(refused.clone()))
            .send()
            .await
            .unwrap();
        let problem = serde_json::to_value(
            error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "configuration_invalid",
            )
            .await
            .problem,
        )
        .unwrap();
        let issues = problem["params"]["issues"].as_array().unwrap();
        let issue = issues
            .iter()
            .find(|issue| issue["code"] == "dns_resolver_plan_incomplete")
            .unwrap_or_else(|| panic!("{endpoint}: {problem}"));
        assert_eq!(issue["path"], "/dns/searchDomains/0", "{endpoint}");
        assert_eq!(issue["params"]["node"], node.to_string());
        assert_eq!(issue["params"]["profile"], "mesh");
        assert_eq!(
            issue["params"]["reason"], "search_domain_without_resolver",
            "{endpoint}"
        );
    }
    assert_eq!(source(&c, &s, n, token).await.revision, initial.revision);
    let answered = serde_json::json!({
        "servers": {"office": {"type": "udp", "server": "10.20.0.53"}},
        "routes": {"churn1.m1.test": ["office"]},
        "searchDomains": ["churn1.m1.test"]
    });
    for endpoint in ["configuration/validate", "configuration"] {
        c.post(net(&s, n, endpoint))
            .bearer_auth(token)
            .json(&request_for(answered.clone()))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let saved = source(&c, &s, n, token).await;
    assert_eq!(saved.configuration.dns.search_domains, ["churn1.m1.test"]);
    s.close().await;
}
