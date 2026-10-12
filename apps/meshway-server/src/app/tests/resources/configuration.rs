use super::*;

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
    let mut input = current.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (_, _, _) = enroll(&s, token, network, 51).await;
    let (b, _, _) = enroll(&s, token, network, 52).await;
    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
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
