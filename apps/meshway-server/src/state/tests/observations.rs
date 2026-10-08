use super::*;

#[test]
fn observations_bind_instance_path_and_independent_subscriber_leases() {
    let (report, _, scope, now) = setup();
    let mut admission = TaskAdmission {
        configuration: report.execution.clone().unwrap(),
        paths: report.current().unwrap().paths.clone(),
        node_expires_at: report.lease_expires_at,
        observation_id: None,
        unsupported_metrics: vec![],
        report: report.clone(),
        principal: crate::observations::SubscriberPrincipal::UserSession {
            session: id(10),
            identity: identity(report.network_id),
        },
        authorization_digest: scope.authorization_digest,
        authorization_expires_at: now + Duration::minutes(10),
    };
    let request = CreateObservation {
        service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
        subscription_id: id(100),
        source_node_id: report.node_id,
        target_node_id: id(2),
        selection: ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    let mut observations = Observations::default();
    let first = observations
        .create(
            &admission,
            identity(report.network_id),
            request.clone(),
            now,
        )
        .unwrap();
    assert!(first.latest.is_none());
    assert!(first.path.is_none());
    admission.principal = crate::observations::SubscriberPrincipal::UserSession {
        session: id(11),
        identity: identity(report.network_id),
    };
    let second = observations
        .create(
            &admission,
            identity(report.network_id),
            CreateObservation {
                subscription_id: id(101),
                ..request
            },
            now,
        )
        .unwrap();
    assert_eq!(first.id, second.id);
    observations
        .cancel(id(10), report.network_id, first.id, id(100), now)
        .unwrap();
    assert_eq!(observations.node_tasks(&report, now).len(), 1);
    assert!(
        observations
            .node_tasks(&report, now + Duration::seconds(31))
            .is_empty()
    );
    assert!(
        observations
            .renew(&admission, first.id, id(101), now + Duration::seconds(31))
            .is_err()
    );
}

#[test]
fn observation_authorization_distinguishes_user_session_from_execution_and_scope_loss() {
    let (report, _, scope, now) = setup();
    let admission = TaskAdmission {
        configuration: report.execution.clone().unwrap(),
        paths: report.current().unwrap().paths.clone(),
        node_expires_at: report.lease_expires_at,
        observation_id: None,
        unsupported_metrics: vec![],
        report: report.clone(),
        principal: crate::observations::SubscriberPrincipal::UserSession {
            session: id(10),
            identity: identity(report.network_id),
        },
        authorization_digest: scope.authorization_digest,
        authorization_expires_at: now + Duration::minutes(10),
    };
    let request = CreateObservation {
        service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
        subscription_id: id(100),
        source_node_id: report.node_id,
        target_node_id: id(2),
        selection: ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    for case in 0..5 {
        let mut tasks = Observations::default();
        let task = tasks
            .create(
                &admission,
                identity(report.network_id),
                request.clone(),
                now,
            )
            .unwrap();
        let mut changed = admission.clone();
        let expected = match case {
            0 => {
                changed.node_expires_at = now;
                Error::StateConflict
            }
            1 => {
                changed.report.instance_id = id(99);
                Error::StateConflict
            }
            2 => {
                changed.configuration.execution_publication = id(99);
                Error::StateConflict
            }
            3 => {
                changed.authorization_digest = Digest::from_bytes([99; 32]);
                Error::PermissionDenied
            }
            _ => {
                changed.authorization_expires_at = now;
                Error::InvalidSession
            }
        };
        assert_eq!(
            tasks
                .renew(
                    &changed,
                    task.id,
                    request.subscription_id,
                    now + Duration::seconds(10)
                )
                .unwrap_err(),
            expected
        );
    }
    let mut expired_node = admission;
    expired_node.node_expires_at = now;
    assert_eq!(
        Observations::default()
            .create(&expired_node, identity(report.network_id), request, now)
            .unwrap_err(),
        Error::StateConflict
    );
}

#[test]
fn shared_task_subscribers_renew_independently_and_create_retry_does_not_renew() {
    let (report, _, scope, now) = setup();
    let mut admission = TaskAdmission {
        configuration: report.execution.clone().unwrap(),
        paths: report.current().unwrap().paths.clone(),
        node_expires_at: report.lease_expires_at,
        observation_id: None,
        unsupported_metrics: vec![],
        report: report.clone(),
        principal: crate::observations::SubscriberPrincipal::UserSession {
            session: id(10),
            identity: identity(report.network_id),
        },
        authorization_digest: scope.authorization_digest,
        authorization_expires_at: now + Duration::minutes(5),
    };
    let request = CreateObservation {
        service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
        subscription_id: id(100),
        source_node_id: report.node_id,
        target_node_id: id(2),
        selection: ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    let mut tasks = Observations::default();
    let first = tasks
        .create(
            &admission,
            identity(report.network_id),
            request.clone(),
            now,
        )
        .unwrap();
    let retry = tasks
        .create(
            &admission,
            identity(report.network_id),
            request.clone(),
            now + Duration::seconds(6),
        )
        .unwrap();
    assert_eq!(retry.subscriber_expires_at, first.subscriber_expires_at);
    admission.principal = crate::observations::SubscriberPrincipal::UserSession {
        session: id(11),
        identity: identity(report.network_id),
    };
    let second = tasks
        .create(
            &admission,
            identity(report.network_id),
            CreateObservation {
                subscription_id: id(101),
                ..request
            },
            now + Duration::seconds(5),
        )
        .unwrap();
    assert_eq!(first.id, second.id);
    admission.principal = crate::observations::SubscriberPrincipal::UserSession {
        session: id(10),
        identity: identity(report.network_id),
    };
    tasks
        .renew(&admission, first.id, id(100), now + Duration::seconds(10))
        .unwrap();
    admission.principal = crate::observations::SubscriberPrincipal::UserSession {
        session: id(11),
        identity: identity(report.network_id),
    };
    let renewed = tasks
        .renew(&admission, second.id, id(101), now + Duration::seconds(15))
        .unwrap();
    assert_eq!(
        renewed.subscriber_expires_at,
        Some(now + Duration::seconds(45))
    );
    tasks
        .cancel(
            id(11),
            report.network_id,
            first.id,
            id(101),
            now + Duration::seconds(16),
        )
        .unwrap();
    assert_eq!(
        tasks.node_tasks(&report, now + Duration::seconds(16))[0].expires_at,
        now + Duration::seconds(40)
    );
}

#[test]
fn typed_channel_reports_and_reselection_keep_authority_and_segment_high_water() {
    // Explicit source-report input tests acceptance only; no packet is emitted.
    use meshway_model::observations::{
        LinkAcquisition, ObservationSample, ProbeChannel, ProbeChannels,
    };
    let (mut report, _, scope, now) = setup();
    let admission = TaskAdmission {
        observation_id: None,
        unsupported_metrics: vec![],
        report: report.clone(),
        configuration: report.execution.clone().unwrap(),
        paths: report.current().unwrap().paths.clone(),
        node_expires_at: report.lease_expires_at,
        principal: crate::observations::SubscriberPrincipal::UserSession {
            session: id(10),
            identity: identity(report.network_id),
        },
        authorization_digest: scope.authorization_digest,
        authorization_expires_at: report.lease_expires_at,
    };
    let mut tasks = Observations::default();
    let task = tasks
        .create(
            &admission,
            identity(report.network_id),
            CreateObservation {
                subscription_id: id(90),
                source_node_id: report.node_id,
                target_node_id: id(2),
                selection: ObservationPath::Auto,
                service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 1000,
            },
            now,
        )
        .unwrap();
    report.contexts[0].diagnostic_transports.insert(
        task.id,
        BTreeMap::from([(
            vec![report.node_id, id(2)],
            BTreeSet::from(["native_udp".into()]),
        )]),
    );
    let channel = ProbeChannel {
        link_id: Digest::from_bytes([1; 32]),
        socket_id: Digest::from_bytes([2; 32]),
        peer_node_id: id(2),
        transport: meshway_transport_spec::TransportProtocol::NativeUdp,
        local_address: "192.0.2.1:1234".parse().unwrap(),
        remote_address: "192.0.2.2:1234".parse().unwrap(),
        acquisition: LinkAcquisition::PublishedEndpoint,
    };
    let probe = ProbeObservation {
        measurement_id: id(91),
        kind: ProbeKind::Path,
        link_id: None,
        transport: None,
        channels: Some(ProbeChannels {
            request: channel.clone(),
            reply: Some(channel),
        }),
        path: vec![report.node_id, id(2)],
        publication_id: task.configuration.execution_publication,
        started_at: now,
        completed_at: Some(now + Duration::milliseconds(1)),
        outcome: ProbeOutcome::Success,
        duration_us: Some(Sequence::new(1000).unwrap()),
    };
    let mut wildcard = probe.clone();
    wildcard.channels.as_mut().unwrap().request.local_address = "0.0.0.0:1234".parse().unwrap();
    validation::validate_probe(&report, &wildcard, now).unwrap();
    for case in 0..7 {
        let mut invalid = probe.clone();
        let channels = invalid.channels.as_mut().unwrap();
        match case {
            0 => channels.request.peer_node_id = id(3),
            1 => channels.request.transport = meshway_transport_spec::TransportProtocol::NativeTcp,
            2 => channels.request.remote_address = "100.64.0.9:1234".parse().unwrap(),
            3 => channels.request.local_address = "255.255.255.255:1234".parse().unwrap(),
            4 => channels.request.remote_address = "[::ffff:192.0.2.2]:1234".parse().unwrap(),
            5 => channels.reply = None,
            _ => channels.reply.as_mut().unwrap().peer_node_id = id(3),
        }
        assert_eq!(
            validation::validate_probe(&report, &invalid, now),
            Err(Error::InvalidRequest)
        );
    }
    let sample = ObservationSample {
        measurement_id: probe.measurement_id,
        version: Version::MIN,
        window_start: now,
        window_end: now + Duration::milliseconds(1),
        partial: false,
        probe: Some(probe),
        traffic: None,
        probe_counts: None,
    };
    let result = ObservationResult {
        instance_id: report.instance_id,
        configuration: task.configuration.clone(),
        result: ObservationUpdate::Sample {
            path: vec![report.node_id, id(2)],
            path_segment: Sequence::new(1).unwrap(),
            sample: Box::new(sample.clone()),
        },
    };
    let mut unrelated = report.clone();
    unrelated.contexts[0].diagnostic_transports.clear();
    assert_eq!(
        tasks
            .result(
                &unrelated,
                task.id,
                result.clone(),
                now + Duration::milliseconds(1)
            )
            .unwrap_err(),
        Error::InvalidRequest
    );
    tasks
        .result(
            &report,
            task.id,
            result.clone(),
            now + Duration::milliseconds(1),
        )
        .unwrap();
    let preparing = tasks
        .result(
            &report,
            task.id,
            ObservationResult {
                result: ObservationUpdate::Phase {
                    phase: ReportedObservationPhase::Preparing,
                },
                ..result.clone()
            },
            now + Duration::milliseconds(2),
        )
        .unwrap();
    assert_eq!(preparing.phase, ObservationPhase::Preparing);
    assert!(preparing.latest.is_none() && preparing.path.is_none());
    assert_eq!(preparing.path_segment, Some(Sequence::new(1).unwrap()));
    assert_eq!(
        tasks
            .result(
                &report,
                task.id,
                result.clone(),
                now + Duration::milliseconds(3)
            )
            .unwrap_err(),
        Error::StateConflict
    );
    let mut next = sample;
    next.measurement_id = id(92);
    next.window_start = now + Duration::milliseconds(2);
    next.window_end = now + Duration::milliseconds(3);
    let probe = next.probe.as_mut().unwrap();
    probe.measurement_id = next.measurement_id;
    probe.started_at = next.window_start;
    probe.completed_at = Some(next.window_end);
    let resumed = tasks
        .result(
            &report,
            task.id,
            ObservationResult {
                result: ObservationUpdate::Sample {
                    path: vec![report.node_id, id(2)],
                    path_segment: Sequence::new(2).unwrap(),
                    sample: Box::new(next),
                },
                ..result
            },
            now + Duration::milliseconds(3),
        )
        .unwrap();
    assert_eq!(resumed.phase, ObservationPhase::Running);
    assert_eq!(resumed.path_segment, Some(Sequence::new(2).unwrap()));
}
