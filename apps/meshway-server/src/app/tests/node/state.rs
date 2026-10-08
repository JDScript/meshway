use super::*;

#[tokio::test]
async fn a_node_may_report_the_configuration_it_saved_just_before_the_next_revisions() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, ak, _, signer_a, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let service = app.execution();
    let a_session = session(&service, network, a, &ak, &signer_a).await;
    let ai = service
        .acquire(
            a_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let saved = ai.target.clone().unwrap();
    // Two more revisions land before the node reports what it saved.
    for remark in ["first", "second"] {
        let revision = app
            .configurations()
            .read(owner.token.expose(), network, None)
            .await
            .unwrap()
            .revision;
        app.configurations()
            .apply(
                owner.token.expose(),
                network,
                meshway_protocol::configuration::ConfigurationRequest::Operations {
                    expected_revision: revision,
                    request_id: rid(),
                    ops: vec![ConfigurationOperation::SetNodeSettings {
                        node_id: a,
                        fields: meshway_configuration::NodeSettingsPatch {
                            remark: meshway_configuration::Input::Value(remark.into()),
                            ..Default::default()
                        },
                    }],
                },
                rid(),
            )
            .await
            .unwrap();
    }
    let current = service
        .renew(
            a_session.token.expose(),
            InstanceRequest {
                instance_id: ai.instance_id,
                generation: ai.generation,
            },
        )
        .await
        .unwrap()
        .target
        .unwrap();
    assert_ne!(current, saved);
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let report = s
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            // The saved reference is admitted as history: readable, never an
            // installation or execution authority.
            assert!(report.configurations.contains(&saved));
            assert!(!report.context(&saved).unwrap().installation_allowed);
            assert_eq!(report.execution.as_ref(), Some(&current));
            assert!(report.context(&current).unwrap().installation_allowed);
            // A reference the node never held is still not admitted.
            let invented = ConfigurationRef {
                digest: [9; 32].into(),
                ..saved.clone()
            };
            assert!(matches!(
                report.context(&invented),
                Err(Error::InvalidRequest)
            ));
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn state_admission_uses_real_node_instance_and_current_publication_dependencies() {
    use crate::error::Error;
    use crate::state::service::ReadIdentity;
    use meshway_model::state::ResourceRef;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let a_session = session(&service, network, a, &ak, &signer_a).await;
    let b_session = session(&service, network, b, &bk, &signer_b).await;
    let ai = service
        .acquire(
            a_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    service
        .acquire(
            b_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let instance = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    service
        .demand(
            a_session.token.expose(),
            DemandRequest {
                instance: instance.clone(),
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    let lease = service
        .renew(a_session.token.expose(), instance.clone())
        .await
        .unwrap();
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    let expected = lease.target.clone();
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let owner_digest = crate::accounts::token_digest(owner.token.expose()).unwrap();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let now = chrono::Utc::now();
            let report = s.report_admission_in_transaction(&tx, &token, now).unwrap();
            assert_eq!(report.execution, expected);
            assert_eq!(report.instance_id, ai.instance_id);
            assert!(report.current().unwrap().peers.contains(&b));
            assert!(
                report
                    .current()
                    .unwrap()
                    .paths
                    .iter()
                    .all(|p| p.first() == Some(&a) && p.last() == Some(&b))
            );
            assert!(
                report
                    .current()
                    .unwrap()
                    .resources
                    .contains_key(&ResourceRef::Listener { key: "udp".into() })
            );
            assert!(
                report
                    .current()
                    .unwrap()
                    .resources
                    .iter()
                    .any(|(r, v)| matches!(r, ResourceRef::Dial { .. })
                        && v.configured_endpoints.is_empty())
            );
            let scope = s
                .state_scope_in_transaction(
                    &tx,
                    &ReadIdentity::Node {
                        authorization: token,
                    },
                    now,
                )
                .unwrap();
            assert_eq!(scope.visible, [a, b].into_iter().collect());
            assert_eq!(scope.full, [a].into_iter().collect());
            assert!(
                scope.current[&a]
                    .configurations
                    .contains(report.execution.as_ref().unwrap())
            );
            let identity = ReadIdentity::UserSession {
                digest: owner_digest,
                network,
            };
            let request = meshway_protocol::observations::CreateObservation {
                service: meshway_configuration::execution::ObservationServiceScope::Tcp {
                    port: meshway_transport_spec::Port::new(443).unwrap(),
                },
                subscription_id: ObservationSubscriptionId::new(random_uuid().unwrap()).unwrap(),
                source_node_id: a,
                target_node_id: b,
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a, b],
                },
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            };
            let admitted = crate::observations::service::ObservationProvider::prepare_observation(
                &s, &tx, &identity, &request, now,
            )
            .unwrap();
            assert_eq!(admitted.paths, vec![vec![a, b]]);
            let invalid = meshway_protocol::observations::CreateObservation {
                selection: meshway_protocol::observations::ObservationPath::Explicit {
                    path: vec![a, b, a],
                },
                ..request
            };
            assert!(matches!(
                s.observation_admission_in_transaction(&tx, &identity, &invalid, now),
                Err(Error::PermissionDenied)
            ));
        })
        .await
        .unwrap();
    service
        .acquire(
            a_session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let token = a_session.token.expose().to_owned();
    let s = service.clone();
    let previous = lease.target.unwrap();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let admission = s
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            assert_ne!(admission.instance_id, instance.instance_id);
            // The target the previous instance held is recent history the new
            // instance may still report as saved (it survives on disk), never
            // an installation or execution context.
            assert!(admission.configurations.contains(&previous));
            assert!(!admission.context(&previous).unwrap().installation_allowed);
            assert_ne!(admission.execution.as_ref(), Some(&previous));
            assert!(admission.current().unwrap().peers.is_empty());
            assert!(matches!(
                s.report_admission_in_transaction(
                    &tx,
                    "mws_not-a-node-session",
                    chrono::Utc::now()
                ),
                Err(Error::InvalidCredentials)
            ));
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_multihop_admission_separates_neighbors_paths_and_receipt_history() {
    use meshway_model::state::{ProbeKind, ProbeObservation, ProbeOutcome};
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let csigner = SigningKey::from_bytes(&[63; 32]);
    let c = enroll_tag_node(
        &app,
        owner.token.expose(),
        network,
        &csigner,
        "relay",
        [73; 32],
    )
    .await;
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.defaults.topology = TopologyDefault::Deny;
    config.topology = [(a, c.node_id), (c.node_id, b)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{a}"),
            b: format!("node:{b}"),
        })
        .collect();
    config.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        via: vec![format!("node:{}", c.node_id)],
    }];
    config.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{a}")],
        dst: vec![format!("node:{b}")],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    let s = app.execution();
    let mut sessions = Vec::new();
    let mut instances = Vec::new();
    for (node, keys, signer) in [
        (a, &ak, &asigner),
        (b, &bk, &bsigner),
        (c.node_id, &c.keys, &csigner),
    ] {
        let session = session(&s, network, node, keys, signer).await;
        let lease = s
            .acquire(
                session.token.expose(),
                AcquireInstanceRequest {
                    instance_id: iid(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
        sessions.push(session);
    }
    let request = DemandRequest {
        instance: instances[0].clone(),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    s.demand(sessions[0].token.expose(), request.clone())
        .await
        .unwrap();
    let mut leases = Vec::new();
    for (session, instance) in sessions.iter().zip(&instances) {
        leases.push(
            s.renew(session.token.expose(), instance.clone())
                .await
                .unwrap(),
        );
    }
    let old = leases[0].target.clone().unwrap();
    s.receipt(
        sessions[0].token.expose(),
        ReceiptRequest {
            instance: instances[0].clone(),
            target: old.clone(),
            phase: ReceiptPhase::Saved,
            attempt: Version::MIN,
            sequence: Sequence::MIN,
            reported_at_ms: store::execution::now().unwrap() as u64,
            failure: None,
        },
    )
    .await
    .unwrap();
    let tokens: Vec<_> = sessions
        .iter()
        .map(|session| session.token.expose().to_owned())
        .collect();
    let executor = s.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let now = chrono::Utc::now();
            for (index, node, peer) in [(0, a, c.node_id), (2, c.node_id, b)] {
                let admission = executor
                    .report_admission_in_transaction(&tx, &tokens[index], now)
                    .unwrap();
                let mut probe = ProbeObservation {
                    channels: None,
                    measurement_id: EventId::new(random_uuid().unwrap()).unwrap(),
                    kind: ProbeKind::Neighbor,
                    link_id: Some("reported-channel".into()),
                    transport: Some("native_udp".into()),
                    path: vec![node, peer],
                    publication_id: admission.execution.as_ref().unwrap().execution_publication,
                    started_at: now,
                    completed_at: Some(now),
                    outcome: ProbeOutcome::Success,
                    duration_us: Some(Sequence::new(12).unwrap()),
                };
                crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                // Reproduce the former B projection: business paths alone cannot
                // authorize these adjacent transport probes.
                let mut missing = admission.clone();
                for context in &mut missing.contexts {
                    context.neighbor_transports.clear();
                }
                assert!(crate::state::validation::validate_probe(&missing, &probe, now).is_err());
                probe.kind = ProbeKind::Connect;
                crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                probe.transport = Some("vless".into());
                assert!(crate::state::validation::validate_probe(&admission, &probe, now).is_err());
                probe.kind = ProbeKind::Path;
                probe.transport = None;
                probe.link_id = None;
                assert!(crate::state::validation::validate_probe(&admission, &probe, now).is_err());
                if node == a {
                    probe.path = vec![a, c.node_id, b];
                    crate::state::validation::validate_probe(&admission, &probe, now).unwrap();
                }
            }
        })
        .await
        .unwrap();
    let mut second = request;
    second.request_id = rid();
    s.demand(sessions[0].token.expose(), second).await.unwrap();
    let token = sessions[0].token.expose().to_owned();
    let executor = s.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let admission = executor
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            assert_ne!(admission.execution.as_ref(), Some(&old));
            assert!(admission.current().unwrap().installation_allowed);
            assert!(!admission.context(&old).unwrap().installation_allowed);
            assert_eq!(
                admission.context(&old).unwrap().paths,
                vec![vec![a, c.node_id, b]]
            );
        })
        .await
        .unwrap();
    let rejected = s
        .demand(
            sessions[2].token.expose(),
            DemandRequest {
                instance: instances[2].clone(),
                request_id: rid(),
                target: BusinessTarget::Mesh { node: b },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 60,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        rejected.decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::Unauthorized
        }
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn withdrawing_execution_dependencies_commits_and_never_falls_through_to_direct() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, asigner, bsigner) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let current = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.nodes.get_mut(&b).unwrap().exit_enabled = true;
    config.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["1.1.1.1/32".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    let rule_sets = BTreeMap::from([(
        "destinations".into(),
        RuleSet::Inline {
            ip_cidr: vec!["1.1.1.1/32".into()],
            domain_suffix: vec![],
            domain: vec![],
        },
    )]);
    config.rule_sets = rule_sets.clone();
    config.routing.egress.rules = vec![
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                rule_sets: vec!["destinations".into()],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        },
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["1.1.1.1/32".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        },
    ];
    let applied = app
        .configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
    let mut revision = applied.revision;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &asigner).await;
    let bt = session(&service, network, b, &bk, &bsigner).await;
    let ai = service
        .acquire(
            at.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    service
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let request = DemandRequest {
        instance: InstanceRequest {
            instance_id: ai.instance_id,
            generation: ai.generation,
        },
        request_id: rid(),
        target: BusinessTarget::Ip {
            address: ExecutionAddress("1.1.1.1".parse().unwrap()),
        },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    assert!(matches!(
        service
            .demand(at.token.expose(), request.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Permitted { .. }
    ));
    for (ops, reason) in [
        (
            vec![ConfigurationOperation::ReplaceAccess { access: vec![] }],
            Some(UnavailableReason::Unauthorized),
        ),
        (
            vec![ConfigurationOperation::ReplaceAccess {
                access: config.access.clone(),
            }],
            None,
        ),
        (
            vec![ConfigurationOperation::SetNodeSettings {
                node_id: b,
                fields: NodeSettingsPatch {
                    exit_enabled: Input::Value(false),
                    ..Default::default()
                },
            }],
            Some(UnavailableReason::NoPath),
        ),
        (
            vec![ConfigurationOperation::DeleteNode { node_id: b }],
            Some(UnavailableReason::NoPath),
        ),
    ] {
        let update = || meshway_protocol::configuration::ConfigurationRequest::Operations {
            expected_revision: revision,
            request_id: rid(),
            ops: ops.clone(),
        };
        let preview = app
            .configurations()
            .validate(owner.token.expose(), network, update(), rid())
            .await
            .unwrap();
        if reason.is_some() {
            assert!(!preview.unavailable_dependencies.is_empty());
        }
        revision = app
            .configurations()
            .apply(owner.token.expose(), network, update(), rid())
            .await
            .unwrap()
            .revision;
        let decision = service
            .demand(at.token.expose(), request.clone())
            .await
            .unwrap()
            .decision;
        match reason {
            Some(reason) => assert_eq!(decision, DemandDecision::Unavailable { reason }),
            None => assert!(matches!(decision, DemandDecision::Permitted { .. })),
        }
    }
    assert_eq!(
        service.authenticate(bt.token.expose()).await.err(),
        Some(Error::InvalidCredentials)
    );
    let after = app
        .configurations()
        .read(owner.token.expose(), network, None)
        .await
        .unwrap();
    assert_eq!(after.document.configuration.routing, config.routing);
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
