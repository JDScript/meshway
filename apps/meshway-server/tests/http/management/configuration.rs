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
    let mut input = meshway_configuration::NetworkConfigurationInput::from(&initial.configuration);
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
        let op = serde_json::from_str(&format!(r#"{{"type":"replaceDefaults","defaults":{{"dialers":{{"vless":{{"priority":17,"websocket":{websocket}}}}}}}}}"#)).unwrap();
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
        assert_eq!(vless.priority, 17);
        assert_eq!(vless.websocket.unwrap().headers.len(), expected_count);
    }
    s.close().await;
}

#[tokio::test]
async fn join_roles_configuration_cas_comment_preservation_and_revocation() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "owner@example.test", "valid password").await;
    let member = register(&c, &s, "member@example.test", "valid password").await;
    let network = create(&c, &s, owner.token.expose(), "network")
        .await
        .network;
    let outsider = create(&c, &s, member.token.expose(), "other").await.network;
    reject(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(member.token.expose()),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let creation = CreateJoinLinkRequest {
        request_id: request(),
        role: MembershipRole::Member,
        expires_in_seconds: 600,
    };
    let response = c
        .post(net(&s, network.id, "join-links"))
        .bearer_auth(owner.token.expose())
        .json(&creation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: JoinLinkCreated = response.json().await.unwrap();
    let JoinLinkCreated::Available { link, secret, .. } = created else {
        panic!("first delivery must contain secret")
    };
    let retry: JoinLinkCreated = json(
        c.post(net(&s, network.id, "join-links"))
            .bearer_auth(owner.token.expose())
            .json(&creation),
    )
    .await;
    assert!(matches!(retry, JoinLinkCreated::Unavailable { .. }));
    let inspection: JoinLinkInspection = json(
        c.post(format!("{}/api/v1/join-links/inspect", s.url))
            .bearer_auth(member.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            }),
    )
    .await;
    assert_eq!(inspection.link.id, link.id);
    let joined: JoinLinkRedeemed = json(
        c.post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(member.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            }),
    )
    .await;
    assert_eq!(joined.network_id, network.id);
    reject(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(member.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    reject(
        c.post(net(
            &s,
            outsider.id,
            &format!("members/{}/role", joined.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        }),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    reject(
        c.post(net(
            &s,
            network.id,
            &format!("members/{}/role", network.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        }),
        StatusCode::CONFLICT,
        "last_owner",
    )
    .await;
    let promoted: MemberMutationResponse = json(
        c.post(net(
            &s,
            network.id,
            &format!("members/{}/role", joined.membership_id),
        ))
        .bearer_auth(owner.token.expose())
        .json(&ChangeMemberRoleRequest {
            expected_version: meshway_model::Version::MIN,
            role: MembershipRole::Admin,
        }),
    )
    .await;
    assert_eq!(promoted.member.version.get(), 2);
    let source: ConfigurationSourceResponse = json(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    let doc = ConfigurationRequest::Document {
        expected_revision: Revision::MIN,
        request_id: request(),
        source_jsonc: WireSecret::new(format!(
            "// retained comment\n// {}\n{}",
            "long configuration comment ".repeat(1000),
            source.source_jsonc.expose()
        )),
    };
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, network.id, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&doc),
    )
    .await;
    assert_eq!(applied.revision.get(), 2);
    assert_eq!(applied.target_count, 0);
    let edit = ConfigurationRequest::Operations {
        expected_revision: applied.revision,
        request_id: request(),
        ops: vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
    };
    let preview: ConfigurationPreviewResponse = json(
        c.post(net(&s, network.id, "configuration/validate"))
            .bearer_auth(member.token.expose())
            .json(&edit),
    )
    .await;
    assert_eq!(preview.change_count, 1);
    assert!(preview.preview.access.is_empty());
    let applied: ConfigurationAppliedResponse = json(
        c.post(net(&s, network.id, "configuration"))
            .bearer_auth(member.token.expose())
            .json(&edit),
    )
    .await;
    assert_eq!(applied.revision.get(), 3);
    let retry: ConfigurationAppliedResponse = json(
        c.post(net(&s, network.id, "configuration"))
            .bearer_auth(member.token.expose())
            .json(&edit),
    )
    .await;
    assert_eq!(retry.revision, applied.revision);
    reject(
        c.post(net(&s, network.id, "configuration/validate"))
            .bearer_auth(member.token.expose())
            .json(&edit),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    let source: ConfigurationSourceResponse = json(
        c.get(net(&s, network.id, "configuration/source"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert!(
        source
            .source_jsonc
            .expose()
            .starts_with("// retained comment")
    );
    let history: ConfigurationRevisionList = json(
        c.get(net(&s, network.id, "configuration/revisions?limit=1"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(history.items[0].revision.get(), 3);
    assert!(history.next_cursor.is_some());
    let restored: ConfigurationAppliedResponse = json(
        c.post(net(&s, network.id, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Restore {
                expected_revision: applied.revision,
                resource_publication: None,
                request_id: request(),
                restore_from: Revision::MIN,
            }),
    )
    .await;
    assert_eq!(restored.revision.get(), 4);
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
