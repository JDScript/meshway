use super::*;

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
