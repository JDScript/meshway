use super::*;

#[tokio::test]
async fn configuration_prepares_new_and_replaced_references_in_one_cas_without_preview_writes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("apply-resources@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (a, at, ai) = enroll(&s, token, network, 41).await;
    let (b, _, _) = enroll(&s, token, network, 42).await;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.rule_sets.insert("remote".into(), remote.definition());
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["8.8.8.0/24".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let before = resource_counts(&s, network).await;
    let preview: ConfigurationPreviewResponse = json(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    assert_eq!(preview.prepared_rule_sets["remote"].entries, 1);
    assert_eq!(resource_counts(&s, network).await, before);
    assert_eq!(s.source(token, network).await.revision, current.revision);
    let applied = s.apply(token, network, current.revision, &input).await;
    assert_eq!(applied.revision.get(), current.revision.get() + 1);
    let first = s.view(token, network).await.artifact.unwrap().id;
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first
    );
    let demand_id = rid();
    let result: DemandResponse = json(
        s.client
            .post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: ai.clone(),
                request_id: demand_id,
                target: BusinessTarget::Ip {
                    address: ExecutionAddress("8.8.8.200".parse().unwrap()),
                },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 300,
            }),
    )
    .await;
    assert!(matches!(
        result.decision,
        DemandDecision::Unmanaged | DemandDecision::Direct
    ));
    let current = s.source(token, network).await;
    let mut replacement = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = replacement.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/replacement");
    }
    remote.set(200, "payload:\n - 0.0.0.0/0\n");
    let before = resource_counts(&s, network).await;
    for endpoint in ["configuration/validate", "configuration"] {
        let response = s
            .client
            .post(s.network(network, endpoint))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&replacement).unwrap()),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(resource_counts(&s, network).await, before);
        assert_eq!(
            s.source(token, network).await.source_jsonc.expose(),
            current.source_jsonc.expose()
        );
    }
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let replaced = s
        .apply(token, network, current.revision, &replacement)
        .await;
    let second = s.view(token, network).await.artifact.unwrap().id;
    assert_ne!(first, second);
    let target = target(&s, &at, &ai).await.full.unwrap().configuration;
    assert!(matches!(
        target.decisions[&demand_id],
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    ));
    assert_eq!(
        publication_bindings(&s, network, target.publication).await["remote"],
        second
    );
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first
    );

    // A normal concurrent source edit wins its CAS; the downloaded candidate
    // cannot overwrite that newer complete configuration.
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/racing");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: replaced.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    let mut other = NetworkConfigurationInput::from(&current.configuration);
    other.nodes.get_mut(&a.node_id).unwrap().remark = "concurrent edit".into();
    s.apply(token, network, current.revision, &other).await;
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, second);

    // Same source revision, newer bound content: the final transaction also
    // rejects the stale base artifact rather than relying solely on source CAS.
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/artifact-race");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    remote.set(200, "payload:\n - 8.8.8.128/25\n");
    let refreshed = s.refresh(token, network).await.artifact.unwrap().id;
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, refreshed);
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        current.source_jsonc.expose()
    );
    // Both apply and validate recheck the actual manager role after shared I/O.
    use meshway_protocol::management::*;
    let admin = s.register("apply-resource-admin@example.com").await;
    let link: JoinLinkCreated = json(
        s.client
            .post(s.network(network, "join-links"))
            .bearer_auth(token)
            .json(&CreateJoinLinkRequest {
                request_id: rid(),
                role: MembershipRole::Admin,
                expires_in_seconds: 600,
            }),
    )
    .await;
    let JoinLinkCreated::Available { secret, .. } = link else {
        panic!("join secret");
    };
    let joined: JoinLinkRedeemed = json(
        s.client
            .post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(admin.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            }),
    )
    .await;
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/role-race");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(admin.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    let preview = tokio::spawn(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(admin.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = s
        .client
        .post(s.network(network, &format!("members/{}/role", joined.membership_id)))
        .bearer_auth(token)
        .json(&ChangeMemberRoleRequest {
            expected_version: Version::MIN,
            role: MembershipRole::Member,
        })
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        preview.await.unwrap().unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        current.source_jsonc.expose()
    );
    // Per-file valid inputs cannot multiply into an unbounded execution input
    // through aliases or successive configuration versions.
    let mut body = String::from("payload:\n");
    let half = u32::try_from(crate::resources::MAX_RULE_ENTRIES / 2).unwrap();
    for index in 0..=half {
        body.push_str(&format!(
            " - {}/32\n",
            std::net::Ipv4Addr::from(0x0b000000 + index)
        ));
    }
    remote.set(200, &body);
    let mut large = NetworkConfigurationInput::from(&current.configuration);
    let mut definition = remote.definition();
    if let RuleSet::Remote { url, .. } = &mut definition {
        *url = remote.url("/large");
    }
    large.rule_sets.insert("large-a".into(), definition.clone());
    large.rule_sets.insert("large-b".into(), definition);
    let before = resource_counts(&s, network).await;
    for endpoint in ["configuration/validate", "configuration"] {
        let response = s
            .client
            .post(s.network(network, endpoint))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&large).unwrap()),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let error: ErrorResponse = response.json().await.unwrap();
        let Problem::ConfigurationInvalid { params, .. } = error.problem else {
            panic!("typed budget error");
        };
        assert!(
            params
                .issues
                .iter()
                .any(|issue| issue.code == IssueCode::RuleSetBudget
                    && issue.path == "/ruleSets"
                    && issue.params.contains_key("maxEntries"))
        );
        assert_eq!(resource_counts(&s, network).await, before);
    }
    // Too many remote sets in one edit is the same kind of diagnostic, not a
    // server outage.
    let mut many = NetworkConfigurationInput::from(&current.configuration);
    for index in 0..=crate::resources::MAX_REMOTE_RULE_SETS {
        let mut definition = remote.definition();
        if let RuleSet::Remote { url, .. } = &mut definition {
            *url = remote.url(&format!("/many/{index}"));
        }
        many.rule_sets.insert(format!("many-{index}"), definition);
    }
    let response = s
        .client
        .post(s.network(network, "configuration/validate"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&many).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let error: ErrorResponse = response.json().await.unwrap();
    let Problem::ConfigurationInvalid { params, .. } = error.problem else {
        panic!("typed budget error");
    };
    assert_eq!(params.issues.len(), 1);
    assert_eq!(params.issues[0].code, IssueCode::RuleSetBudget);
    assert_eq!(
        params.issues[0].params["maxSets"],
        crate::resources::MAX_REMOTE_RULE_SETS.to_string()
    );
    assert_eq!(resource_counts(&s, network).await, before);
    s.close().await;
}

#[tokio::test]
async fn exact_names_in_remote_lists_bind_under_an_internet_grant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    // A MetaCubeX-style geosite list: suffixes and bare exact names mixed.
    remote.set(
        200,
        "payload:\n - '+.telegram.org'\n - 'telegram.me'\n - 't.me'\n - '+.t.me'\n",
    );
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("exact-names@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (_, _, _) = enroll(&s, token, network, 51).await;
    let (b, _, _) = enroll(&s, token, network, 52).await;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.rule_sets.insert(
        "remote".into(),
        serde_json::from_str(&format!(
            r#"{{"type":"remote","url":"{}","format":"mihomo-domain"}}"#,
            remote.url("/geosite")
        ))
        .unwrap(),
    );
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let preview: ConfigurationPreviewResponse = json(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    assert_eq!(preview.prepared_rule_sets["remote"].entries, 4);
    s.apply(token, network, current.revision, &input).await;
    let view = s.view(token, network).await;
    assert_eq!(view.problem, None);
    let artifact = view.artifact.unwrap();
    assert_eq!(artifact.entries, 4);
    let data = s
        .app
        .executor
        .handle()
        .read(move |tx| crate::resources::load_artifact(tx, network, artifact.id))
        .await
        .unwrap();
    assert_eq!(data.domain_suffix, vec!["t.me", "telegram.org"]);
    assert_eq!(data.domain, vec!["t.me", "telegram.me"]);
    assert!(data.ip_cidr.is_empty());
    // The one mihomo form without an equivalent still fails the whole list.
    remote.set(200, "payload:\n - '*.telegram.org'\n");
    let failed = s.refresh(token, network).await;
    assert_eq!(failed.problem, Some(ResourceProblem::ConversionFailed));
    assert_eq!(failed.artifact.unwrap().id, artifact.id);
    // A refresh is idempotent per request ID: the retry neither downloads
    // nor records a new check; the same ID for another key is a conflict.
    let replay = rid();
    let refresh = |key: &str, request_id: RequestId| {
        s.client
            .post(s.network(network, &format!("rule-sets/{key}/refresh")))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id })
            .send()
    };
    let downloads = remote.requests.load(Ordering::SeqCst);
    let first: RuleSetView = refresh("remote", replay)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remote.requests.load(Ordering::SeqCst), downloads + 1);
    tokio::time::sleep(Duration::from_millis(5)).await;
    let again: RuleSetView = refresh("remote", replay)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remote.requests.load(Ordering::SeqCst), downloads + 1);
    assert_eq!(again.checked_at, first.checked_at);
    let conflict = refresh("other", replay).await.unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = conflict.json().await.unwrap();
    assert!(matches!(error.problem, Problem::RequestConflict { .. }));
    let missing = s
        .client
        .post(s.network(network, "rule-sets/remote/refresh"))
        .bearer_auth(token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    s.close().await;
}
