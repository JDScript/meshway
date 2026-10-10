use super::*;

#[tokio::test]
async fn remote_https_artifacts_compile_publish_restore_and_survive_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let probe = remote
        .client(
            "rules.meshway.example",
            &[SocketAddr::new(
                "93.184.216.34".parse().unwrap(),
                remote.address.port(),
            )],
        )
        .unwrap()
        .get(remote.url("/rules"))
        .send()
        .await
        .expect("fixture TLS must verify");
    assert_eq!(probe.text().await.unwrap(), "payload:\n - 8.8.8.0/25\n");
    let owner = s.register("resource-owner@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let source = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.rule_sets.insert("remote".into(), remote.definition());
    s.apply(token, network, source.revision, &input).await;
    assert!(s.view(token, network).await.problem.is_none());
    let (a, at, ai) = enroll(&s, token, network, 31).await;
    let (b, bt, bi) = enroll(&s, token, network, 32).await;
    let (c, ct, ci) = enroll(&s, token, network, 33).await;
    let source = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.topology.push(TopologyRule {
        transports: Input::Omitted,
        remark: String::new(),
        action: TopologyAction::Block,
        a: format!("node:{}", c.node_id),
        b: "*".into(),
        keepalive: false,
    });
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
    let response = s
        .client
        .post(s.network(network, "configuration/validate"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: source.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    let first = s.refresh(token, network).await;
    assert!(first.problem.is_none(), "{:?}", first.problem);
    let first_id = first.artifact.unwrap().id;
    let applied = s.apply(token, network, source.revision, &input).await;
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first_id
    );
    let fixed_revision = applied.revision;
    let source_before = s.source(token, network).await;
    let demand_id = rid();
    let demand: DemandResponse = json(
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
        demand.decision,
        DemandDecision::Unmanaged | DemandDecision::Direct
    ));
    let _: InstanceLease = json(
        s.client
            .post(format!("{}/agent-api/v1/instances/renew", s.url))
            .bearer_auth(&ct)
            .json(&ci),
    )
    .await;
    let unrelated = target(&s, &ct, &ci).await;
    let response = s
        .client
        .post(format!("{}/agent-api/v1/configuration/receipts", s.url))
        .bearer_auth(&ct)
        .json(&ReceiptRequest {
            instance: ci.clone(),
            target: unrelated.lease.target.unwrap(),
            phase: ReceiptPhase::Saved,
            attempt: Version::MIN,
            sequence: Sequence::MIN,
            reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
            failure: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let unrelated_facts = execution_facts(&s, network, c.node_id).await;
    assert!(unrelated_facts.1.is_some());
    assert_eq!(unrelated_facts.2, 1);
    // Subscribe through the real stream, finish its initial batch, then refresh.
    let mut events = s
        .client
        .get(s.network(network, "events"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let mut stream_text = String::new();
    while !stream_text.contains("sync.ready") {
        stream_text.push_str(std::str::from_utf8(&events.chunk().await.unwrap().unwrap()).unwrap());
    }
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let second = s.refresh(token, network).await;
    assert!(second.problem.is_none());
    let last_success = second.last_success_at;
    assert!(last_success.is_some());
    let second_publication = second.publication.unwrap();
    let second_id = second.artifact.unwrap().id;
    assert_ne!(first_id, second_id);
    // Even the isolated node must receive the expanded common capture range;
    // lack of a usable path cannot leave the added range locally unmanaged.
    assert_ne!(
        execution_facts(&s, network, c.node_id).await,
        unrelated_facts
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !stream_text.contains("resources.changed") {
            stream_text
                .push_str(std::str::from_utf8(&events.chunk().await.unwrap().unwrap()).unwrap());
        }
    })
    .await
    .unwrap();
    assert!(
        !stream_text
            .lines()
            .find(|line| line.contains("resources.changed"))
            .unwrap()
            .contains("rules.meshway.example")
    );
    drop(events);
    let after = target(&s, &at, &ai).await;
    let configuration = after.full.unwrap().configuration;
    assert!(
        !serde_json::to_string(&configuration)
            .unwrap()
            .contains("rules.meshway.example")
    );
    assert!(matches!(
        configuration.decisions[&demand_id],
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    ));
    assert!(configuration.permits.is_empty());
    assert!(
        configuration
            .egress_capture
            .managed_ipv4
            .iter()
            .any(|prefix| prefix
                .parse::<ipnet::IpNet>()
                .unwrap()
                .contains(&"8.8.8.200".parse::<std::net::IpAddr>().unwrap()))
    );
    assert_eq!(
        publication_bindings(&s, network, configuration.publication).await["remote"],
        second_id
    );
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        source_before.source_jsonc.expose()
    );
    assert_eq!(s.source(token, network).await.revision, fixed_revision);
    // Failed content conversion and widening do not overwrite the last accepted
    // target, source version, immutable content, or historical binding.
    for (status, body, problem) in [
        (503, "unavailable", ResourceProblem::DownloadFailed),
        (
            200,
            "payload:\n - not-a-cidr\n",
            ResourceProblem::ConversionFailed,
        ),
        (
            200,
            "payload:\n - 0.0.0.0/0\n",
            ResourceProblem::ValidationFailed,
        ),
        (302, "redirect", ResourceProblem::DownloadFailed),
    ] {
        remote.set(status, body);
        let failed = s.refresh(token, network).await;
        assert_eq!(failed.last_success_at, last_success);
        assert!(failed.checked_at >= last_success);
        assert_eq!(failed.problem, Some(problem));
        assert_eq!(failed.artifact.unwrap().id, second_id);
        assert_eq!(s.view(token, network).await.problem, Some(problem));
    }
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first_id
    );
    let before_unrelated = target(&s, &bt, &bi).await;
    assert!(before_unrelated.lease.target.is_some());
    // Default restore uses initial bindings, never an implicit latest download.
    // A caller can explicitly select an exact historical execution publication.
    let source = s.source(token, network).await;
    let mut changed = NetworkConfigurationInput::from(&source.configuration);
    changed.nodes.get_mut(&a.node_id).unwrap().remark = "new config".into();
    let current = s.apply(token, network, source.revision, &changed).await;
    assert_eq!(s.view(token, network).await.last_success_at, last_success);
    remote.set(200, "payload:\n - 8.8.8.0/25\n");
    assert_eq!(
        s.refresh(token, network).await.artifact.unwrap().id,
        first_id
    );
    let initial_restore: ConfigurationAppliedResponse = json(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Restore {
                expected_revision: current.revision,
                request_id: rid(),
                restore_from: fixed_revision,
                rule_set_bindings: None,
            }),
    )
    .await;
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, first_id);
    let restored: ConfigurationAppliedResponse = json(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Restore {
                expected_revision: initial_restore.revision,
                request_id: rid(),
                restore_from: fixed_revision,
                rule_set_bindings: Some(second_publication),
            }),
    )
    .await;
    assert_eq!(
        publication_bindings(&s, network, restored.publication_ids[0]).await["remote"],
        second_id
    );
    // A restored exact binding is not immediately replaced just because its
    // new source version has no download check record. Its declared interval
    // starts at this source commit without claiming a fresh download in State.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(s.view(token, network).await.checked_at.is_none());
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, second_id);
    let source = s.source(token, network).await;
    let mut disabled = NetworkConfigurationInput::from(&source.configuration);
    disabled.nodes.get_mut(&b.node_id).unwrap().enabled = false;
    s.apply(token, network, source.revision, &disabled).await;
    let blocked = target(&s, &at, &ai).await.full.unwrap().configuration;
    assert!(matches!(
        blocked.decisions[&demand_id],
        DemandDecision::Unavailable { .. }
    ));
    assert!(
        blocked.permits.is_empty(),
        "missing mandatory exit cannot become direct fallback"
    );
    s.app.check_storage().await.unwrap();
    s.close().await;
    let restarted = Server::open(&path, remote.clone()).await;
    assert_eq!(
        restarted.view(token, network).await.artifact.unwrap().id,
        second_id
    );
    assert_eq!(
        publication_bindings(&restarted, network, restored.publication_ids[0]).await["remote"],
        second_id
    );
    restarted.app.check_storage().await.unwrap();
    restarted.close().await;
}

#[tokio::test]
async fn evaluation_plans_current_and_draft_configurations_with_bound_rule_sets() {
    use meshway_configuration::evaluation::*;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    remote.set(200, "payload:\n - '+.telegram.org'\n");
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("evaluate-owner@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (a, _, _) = enroll(&s, token, network, 61).await;
    let (b, _, _) = enroll(&s, token, network, 62).await;
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
    let applied = s.apply(token, network, current.revision, &input).await;
    let evaluate = |body: serde_json::Value| {
        s.client
            .post(s.network(network, "configuration/evaluate"))
            .bearer_auth(token)
            .json(&body)
            .send()
    };
    // The current version: a remote name is routed to the exit over the
    // bound artifact, and the plan names the rule, the set and the entry.
    let response: EvaluationResponse = evaluate(serde_json::json!({
        "configuration": {"type": "current"},
        "source": a.node_id,
        "target": {"type": "domain", "name": "Web.Telegram.org"}
    }))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(response.revision, applied.revision);
    assert!(!response.draft);
    let e = response.evaluation;
    assert_eq!(
        e.destination,
        EvaluatedDestination::Domain {
            name: "web.telegram.org".into()
        }
    );
    let route = e.route.unwrap();
    assert_eq!(route.kind, RouteKind::Exit);
    assert_eq!(route.rule.as_deref(), Some("/routing/egress/rules/0"));
    assert_eq!(
        route.rule_set,
        Some(RuleSetMatch {
            key: "remote".into(),
            entry: "telegram.org".into()
        })
    );
    assert_eq!(route.targets, vec![b.node_id]);
    assert!(e.access.allowed);
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    assert!(e.paths.is_empty());
    assert_eq!(e.targets[0].problem, Some(TargetProblem::ExitUnavailable));
    // A Mesh peer by address: the template Access rule, automatic routing.
    let b_address = s.source(token, network).await.configuration.nodes[&b.node_id]
        .addresses
        .ipv4
        .unwrap()
        .to_string();
    let response: EvaluationResponse = evaluate(serde_json::json!({
        "configuration": {"type": "current"},
        "source": a.node_id,
        "target": {"type": "address", "address": b_address}
    }))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let e = response.evaluation;
    assert_eq!(
        e.destination,
        EvaluatedDestination::Node { node_id: b.node_id }
    );
    assert_eq!(e.route.as_ref().unwrap().kind, RouteKind::Mesh);
    assert_eq!(e.route.as_ref().unwrap().rule, None);
    assert_eq!(e.access.grants[0].rule, "/access/0");
    assert_eq!(e.outcome, EvaluationOutcome::Permitted);
    // A draft evaluated as a change to the current version, without applying it.
    let mut draft = NetworkConfigurationInput::from(&s.source(token, network).await.configuration);
    draft.nodes.get_mut(&b.node_id).unwrap().exit_enabled = false;
    let before = resource_counts(&s, network).await;
    let response: EvaluationResponse = evaluate(serde_json::json!({
        "configuration": {
            "type": "document",
            "expectedRevision": applied.revision,
            "sourceJsonc": serde_json::to_string(&draft).unwrap()
        },
        "source": a.node_id,
        "target": {"type": "domain", "name": "t.telegram.org"}
    }))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(response.draft);
    assert_eq!(response.revision, applied.revision);
    let e = response.evaluation;
    assert_eq!(e.route.as_ref().unwrap().kind, RouteKind::Exit);
    assert_eq!(e.targets[0].problem, Some(TargetProblem::ExitDisabled));
    assert_eq!(
        e.outcome,
        EvaluationOutcome::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
    assert_eq!(resource_counts(&s, network).await, before);
    assert_eq!(s.source(token, network).await.revision, applied.revision);
    // CAS, unknown source and a profile the source may not use are refused.
    let stale = evaluate(serde_json::json!({
        "configuration": {
            "type": "document",
            "expectedRevision": current.revision,
            "sourceJsonc": serde_json::to_string(&draft).unwrap()
        },
        "source": a.node_id,
        "target": {"type": "internet"}
    }))
    .await
    .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    for body in [
        serde_json::json!({
            "configuration": {"type": "current"},
            "source": "00000000-0000-4000-8000-00000000dead",
            "target": {"type": "internet"}
        }),
        serde_json::json!({
            "configuration": {"type": "current"},
            "source": a.node_id,
            "target": {"type": "internet"},
            "profile": "work"
        }),
        serde_json::json!({
            "configuration": {"type": "current"},
            "source": a.node_id,
            "target": {"type": "address", "address": "not an address"}
        }),
    ] {
        assert_eq!(
            evaluate(body).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    s.close().await;
}
