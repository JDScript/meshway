use super::*;
use meshway_model::{
    exit::{ExitCapability, ExitFamily, ExitStatus, ExitUnavailableReason},
    state::*,
};
use meshway_protocol::state::StateReport;
mod dns;

pub(super) async fn report_current(
    app: &App,
    token: &str,
    lease: &InstanceLease,
    sequence: u64,
    status: ExitStatus,
) -> StateReport {
    let target = app
        .execution()
        .refresh_configuration(
            token,
            NodeConfigurationRequest {
                instance: InstanceRequest {
                    instance_id: lease.instance_id,
                    generation: lease.generation,
                },
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap();
    assert!(target.configuration.exit_role.is_some());
    let reference = ConfigurationRef {
        revision: target.configuration.revision,
        execution_publication: target.configuration.publication,
        digest: target.digest,
    };
    let now = chrono::Utc::now();
    let report = StateReport {
        instance_id: lease.instance_id,
        sequence: Sequence::new(sequence).unwrap(),
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "exit-fixture".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test".into(),
            },
            capabilities: target.configuration.self_keys.capabilities,
            configuration: ConfigurationState::default(),
            exit_capability: Some(ExitCapability {
                instance_id: lease.instance_id,
                target: reference,
                supported_families: vec![ExitFamily::Ipv4],
                installed_families: vec![],
                status,
                observed_at: now,
            }),
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState::default(),
            probes: vec![],
            problems: vec![],
        },
    };
    app.node_state_service()
        .unwrap()
        .accept_report(token.to_owned(), report.clone())
        .await
        .unwrap();
    report
}

#[tokio::test]
async fn exit_capability_before_applied_is_fenced_and_withdraws_joint_permission() {
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
    let mut cfg = current.document.configuration.clone();
    cfg.nodes.get_mut(&b).unwrap().exit_enabled = true;
    cfg.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["tcp:443".into()],
        bidirectional: false,
    }];
    cfg.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            ip_cidr: vec!["8.8.8.8/32".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{b}")],
            via: vec![],
        },
    });
    let request = meshway_protocol::configuration::ConfigurationRequest::Document {
        expected_revision: current.revision,
        request_id: rid(),
        source_jsonc: WireSecret::new(canonical_json(&cfg).unwrap().expose().into()),
    };
    let revision = app
        .configurations()
        .apply(owner.token.expose(), network, request, rid())
        .await
        .unwrap()
        .revision;
    assert!(
        app.configurations()
            .apply(
                owner.token.expose(),
                network,
                meshway_protocol::configuration::ConfigurationRequest::Document {
                    expected_revision: current.revision,
                    request_id: rid(),
                    source_jsonc: WireSecret::new(canonical_json(&cfg).unwrap().expose().into())
                },
                rid()
            )
            .await
            .is_err()
    );
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
    let bi = service
        .acquire(
            bt.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    let d = DemandRequest {
        instance: InstanceRequest {
            instance_id: ai.instance_id,
            generation: ai.generation,
        },
        request_id: rid(),
        target: BusinessTarget::Ip {
            address: ExecutionAddress("8.8.8.8".parse().unwrap()),
        },
        protocol: BusinessProtocol::Tcp,
        port: Some(443.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 120,
    };
    assert_eq!(
        service
            .demand(at.token.expose(), d.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    let failure = report_current(
        &app,
        bt.token.expose(),
        &bi,
        1,
        ExitStatus::Unavailable {
            reason: ExitUnavailableReason::IpForwardDisabled,
        },
    )
    .await;
    let mut changed_sample = failure;
    changed_sample.sequence = Sequence::new(2).unwrap();
    changed_sample
        .state
        .exit_capability
        .as_mut()
        .unwrap()
        .status = ExitStatus::Ready;
    assert!(matches!(
        app.node_state_service()
            .unwrap()
            .accept_report(bt.token.expose().to_owned(), changed_sample)
            .await,
        Err(Error::InvalidRequest)
    ));
    assert_eq!(
        service
            .demand(at.token.expose(), d.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    let inputs = app
        .executor
        .handle()
        .read(move |tx| crate::state::exit::inputs(tx, network, revision, store::execution::now()?))
        .await
        .unwrap();
    assert_eq!(
        inputs.unavailable_reasons.get(&b),
        Some(&ExitUnavailableReason::IpForwardDisabled)
    );
    let ready = report_current(&app, bt.token.expose(), &bi, 2, ExitStatus::Ready).await;
    crate::app::tests::recompile(&app, network).await;
    assert!(matches!(
        service
            .demand(at.token.expose(), d.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Permitted { .. }
    ));
    let mut unknown = ready.clone();
    unknown.sequence = Sequence::new(3).unwrap();
    unknown.state.exit_capability = None;
    app.node_state_service()
        .unwrap()
        .accept_report(bt.token.expose().to_owned(), unknown)
        .await
        .unwrap();
    crate::app::tests::recompile(&app, network).await;
    assert_eq!(
        service
            .demand(at.token.expose(), d.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    let current = service
        .refresh_configuration(
            bt.token.expose(),
            NodeConfigurationRequest {
                instance: InstanceRequest {
                    instance_id: bi.instance_id,
                    generation: bi.generation,
                },
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap();
    let mut reintroduced = ready;
    reintroduced.sequence = Sequence::new(4).unwrap();
    reintroduced.state.exit_capability.as_mut().unwrap().target = ConfigurationRef {
        revision: current.configuration.revision,
        execution_publication: current.configuration.publication,
        digest: current.digest,
    };
    assert!(matches!(
        app.node_state_service()
            .unwrap()
            .accept_report(bt.token.expose().to_owned(), reintroduced)
            .await,
        Err(Error::InvalidRequest)
    ));
    let last = report_current(
        &app,
        bt.token.expose(),
        &bi,
        4,
        ExitStatus::Unavailable {
            reason: ExitUnavailableReason::NftUnavailable,
        },
    )
    .await;
    crate::app::tests::recompile(&app, network).await;
    assert_eq!(
        service
            .demand(at.token.expose(), d.clone())
            .await
            .unwrap()
            .decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::ExitUnavailable
        }
    );
    let mut forged = last.clone();
    forged.sequence = Sequence::new(5).unwrap();
    forged.state.exit_capability.as_mut().unwrap().instance_id = ai.instance_id;
    assert!(matches!(
        app.node_state_service()
            .unwrap()
            .accept_report(bt.token.expose().to_owned(), forged)
            .await,
        Err(Error::InvalidRequest)
    ));
    let mut stale = last;
    stale.sequence = Sequence::new(5).unwrap();
    stale.state.exit_capability.as_mut().unwrap().target.digest = Digest::from_bytes([7; 32]);
    assert!(matches!(
        app.node_state_service()
            .unwrap()
            .accept_report(bt.token.expose().to_owned(), stale)
            .await,
        Err(Error::InvalidRequest)
    ));
    app.configurations()
        .apply(
            owner.token.expose(),
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: revision,
                request_id: rid(),
                ops: vec![ConfigurationOperation::SetNodeSettings {
                    node_id: b,
                    fields: NodeSettingsPatch {
                        exit_enabled: Input::Value(false),
                        ..Default::default()
                    },
                }],
            },
            rid(),
        )
        .await
        .unwrap();
    assert_eq!(
        service.demand(at.token.expose(), d).await.unwrap().decision,
        DemandDecision::Unavailable {
            reason: UnavailableReason::NoPath
        }
    );
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
}
