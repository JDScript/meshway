use super::*;
use crate::observations::{Observations, TaskAdmission};
use crate::resources::content_digest;
use meshway_model::{PublicationId, state::*};
use meshway_protocol::observations::*;
use std::net::IpAddr;

fn id<T: std::str::FromStr>(n: u128) -> T
where
    T::Err: std::fmt::Debug,
{
    uuid::Uuid::from_u128(n).to_string().parse().unwrap()
}
pub(super) fn setup() -> (ReportAdmission, StateReport, VisibilityScope, DateTime<Utc>) {
    let now = Utc::now();
    let node = id(1);
    let peer = id(2);
    let reference = ConfigurationRef {
        revision: Revision::MIN,
        execution_publication: id::<PublicationId>(3),
        digest: content_digest(b"node configuration"),
    };
    let resource = ResourceRef::Listener { key: "udp".into() };
    let admission = ReportAdmission {
        network_id: id(4),
        node_id: node,
        instance_id: id(5),
        instance_generation: Version::MIN,
        lease_expires_at: now + Duration::minutes(5),
        execution: Some(reference.clone()),
        configurations: vec![reference.clone()],
        contexts: vec![ConfigurationAdmission {
            path_deadlines: BTreeMap::from([(vec![node, peer], now + Duration::minutes(5))]),
            configuration: reference.clone(),
            installation_allowed: true,
            resources: BTreeMap::from([(
                resource.clone(),
                ResourceAdmission {
                    network: SocketNetwork::Udp,
                    transport: "native_udp".into(),
                    configured_endpoints: BTreeMap::from([(
                        "/listeners/udp/endpoints/0".into(),
                        "1.1.1.1:9999".into(),
                    )]),
                },
            )]),
            peers: BTreeSet::from([peer]),
            paths: vec![vec![node, peer]],
            neighbor_transports: BTreeMap::from([(peer, BTreeSet::from(["native_udp".into()]))]),
            path_transports: BTreeMap::from([(
                vec![node, peer],
                BTreeSet::from(["native_udp".into()]),
            )]),
            diagnostic_transports: BTreeMap::new(),
            mesh_address_pools: meshway_configuration::AddressPools {
                ipv4: vec!["100.64.0.0/10".into()],
                ipv6: vec![],
            },
            profiles: BTreeSet::from(["mesh".into()]),
        }],
        mesh_addresses: BTreeSet::from(["100.64.0.1".parse::<IpAddr>().unwrap()]),
    };
    let report = StateReport {
        instance_id: admission.instance_id,
        sequence: Sequence::new(1).unwrap(),
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "host".into(),
                os: "linux".into(),
                arch: "arm64".into(),
                agent_version: "test".into(),
            },
            capabilities: BTreeMap::new(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference.clone()),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["s1".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    configuration: None,
                },
            )]),
            sockets: BTreeMap::from([(
                "s1".into(),
                SocketState {
                    resource,
                    configuration: None,
                    generation: Sequence::new(1).unwrap(),
                    network: SocketNetwork::Udp,
                    bound_address: "0.0.0.0:9999".parse().unwrap(),
                    endpoints: vec![EndpointCandidate {
                        address: "1.1.1.1:9999".into(),
                        origin: EndpointSource::Stun {
                            observer: "8.8.8.8:3478".into(),
                        },
                        observed_at: now,
                        expires_at: now + Duration::seconds(10),
                    }],
                },
            )]),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState {
                applied_profile: Some("mesh".into()),
            },
            probes: vec![],
            problems: vec![],
        },
    };
    let scope = VisibilityScope {
        network_id: admission.network_id,
        configuration_revision: Revision::MIN,
        authorization_digest: content_digest(b"authorization"),
        observation_authorization_digest: content_digest(b"authorization"),
        visible: BTreeSet::from([node, peer]),
        hostnames: BTreeMap::from([(node, "node".into()), (peer, "peer".into())]),
        full: BTreeSet::from([node]),
        endpoint_resources: None,
        observation_paths: BTreeSet::from([vec![node, peer]]),
        mesh_addresses: admission.mesh_addresses.clone(),
        expires_at: now + Duration::minutes(10),
        current: BTreeMap::from([(
            node,
            CurrentNode {
                instance_id: Some(admission.instance_id),
                instance_generation: Some(admission.instance_generation),
                execution: Some(reference.clone()),
                configurations: vec![reference],
                revoked: false,
                lease_expires_at: Some(admission.lease_expires_at),
            },
        )]),
    };
    (admission, report, scope, now)
}

#[test]
fn legacy_unicode_socket_ids_remain_full_facts_but_are_not_automatic_references() {
    let (admission, mut report, mut scope, now) = setup();
    let socket = report.state.sockets.remove("s1").unwrap();
    report.state.sockets.insert("旧socket".into(), socket);
    report.state.listeners.get_mut("udp").unwrap().socket_ids = vec!["旧socket".into()];
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    let full = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Full { report }) = &full.nodes[&admission.node_id].report else {
        panic!();
    };
    assert!(report.sockets.contains_key("旧socket"));
    scope.full.clear();
    let peer = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report }) = &peer.nodes[&admission.node_id].report else {
        panic!();
    };
    assert!(report.endpoints.is_empty());
}

#[test]
fn exact_resource_disclosure_and_instance_generation_fence_old_reports() {
    let (mut admission, report, mut scope, now) = setup();
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    scope.full.clear();
    scope.endpoint_resources = Some(BTreeMap::new());
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report: peer }) = &view.nodes[&admission.node_id].report else {
        panic!();
    };
    assert!(peer.endpoints.is_empty());
    scope.endpoint_resources = Some(BTreeMap::from([(
        admission.node_id,
        BTreeSet::from([ResourceRef::Listener { key: "udp".into() }]),
    )]));
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report: peer }) = &view.nodes[&admission.node_id].report else {
        panic!();
    };
    assert_eq!(peer.endpoints.len(), 1);
    assert_eq!(
        peer.endpoints[0]
            .candidate_reference
            .socket
            .instance
            .generation,
        admission.instance_generation
    );
    admission.instance_generation = Version::new(2).unwrap();
    scope
        .current
        .get_mut(&admission.node_id)
        .unwrap()
        .instance_generation = Some(admission.instance_generation);
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    assert_eq!(
        view.nodes[&admission.node_id].freshness,
        Freshness::Superseded
    );
    let mut fresh_report = report;
    fresh_report.sequence = Sequence::new(1).unwrap();
    assert!(
        !hub.accept_report(&admission, fresh_report, now)
            .unwrap()
            .duplicate
    );
}

#[test]
fn freshness_distinguishes_an_absent_instance_from_a_replaced_one() {
    let (admission, report, mut scope, now) = setup();
    let node = admission.node_id;
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    let freshness = |hub: &mut StateHub, scope: &VisibilityScope, at| {
        hub.query(scope, None, &mut GeoIp::default(), at)
            .unwrap()
            .nodes[&node]
            .freshness
    };
    assert_eq!(freshness(&mut hub, &scope, now), Freshness::Current);
    // The instance was released or its lease ran out: the node is gone, not
    // replaced, even though its report is still within its TTL.
    scope.current.get_mut(&node).unwrap().instance_id = None;
    scope.current.get_mut(&node).unwrap().lease_expires_at = None;
    assert_eq!(freshness(&mut hub, &scope, now), Freshness::Expired);
    assert_eq!(
        freshness(&mut hub, &scope, now + Duration::seconds(61)),
        Freshness::Expired
    );
    // A different live instance makes the old report superseded.
    let replacement = InstanceId::new(uuid::Uuid::from_u128(0x77)).unwrap();
    scope.current.get_mut(&node).unwrap().instance_id = Some(replacement);
    scope.current.get_mut(&node).unwrap().lease_expires_at = Some(now + Duration::minutes(2));
    assert_eq!(freshness(&mut hub, &scope, now), Freshness::Superseded);
    // Without any lease the applied reference is not judged against the
    // configuration: no instance, no mismatch.
    scope.current.get_mut(&node).unwrap().instance_id = None;
    scope.current.get_mut(&node).unwrap().configurations.clear();
    assert_eq!(freshness(&mut hub, &scope, now), Freshness::Expired);
}

#[test]
fn retry_is_idempotent_without_refreshing_report_or_endpoint() {
    let (admission, report, scope, now) = setup();
    let mut hub = StateHub::new().unwrap();
    let first = hub.accept_report(&admission, report.clone(), now).unwrap();
    let retry = hub
        .accept_report(&admission, report.clone(), now + Duration::seconds(5))
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(first.received_at, retry.received_at);
    assert_eq!(hub.events.sequence(), 1);
    let mut conflicting = report.clone();
    conflicting.state.system.hostname = "changed".into();
    assert_eq!(
        hub.accept_report(&admission, conflicting, now).unwrap_err(),
        Error::StateConflict
    );
    let mut geo = GeoIp::default();
    let view = hub
        .query(&scope, None, &mut geo, now + Duration::seconds(61))
        .unwrap();
    assert_eq!(view.nodes[&admission.node_id].freshness, Freshness::Expired);
    assert_eq!(view.nodes[&admission.node_id].received_at, Some(now));
    assert_eq!(
        view.nodes[&admission.node_id].last_authenticated_at,
        Some(now + Duration::seconds(5))
    );
}
#[test]
fn snapshot_replacement_deletes_resources_and_stale_instance_rejected() {
    let (admission, mut report, scope, now) = setup();
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    report.sequence = Sequence::new(2).unwrap();
    report.state.sockets.clear();
    report.state.listeners.clear();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Full { report: source }) = &view.nodes[&admission.node_id].report
    else {
        panic!()
    };
    assert!(source.sockets.is_empty());
    report.instance_id = id(99);
    assert_eq!(
        hub.accept_report(&admission, report, now).unwrap_err(),
        Error::InvalidSession
    );
    let mut restarted = StateHub::new().unwrap();
    assert_ne!(restarted.epoch, hub.epoch);
    assert_eq!(
        restarted
            .query(&scope, None, &mut GeoIp::default(), now)
            .unwrap()
            .nodes[&admission.node_id]
            .freshness,
        Freshness::AwaitingReport
    );
}
#[test]
fn configured_udp_and_resource_association_are_not_self_authorized() {
    let (admission, report, _, now) = setup();
    let mut wrong = report.clone();
    wrong.state.sockets.get_mut("s1").unwrap().network = SocketNetwork::Tcp;
    assert!(
        StateHub::new()
            .unwrap()
            .accept_report(&admission, wrong, now)
            .is_err()
    );
    let mut wrong = report.clone();
    wrong.state.sockets.get_mut("s1").unwrap().endpoints[0].origin = EndpointSource::Configured {
        config_path: "/arbitrary".into(),
    };
    assert!(
        StateHub::new()
            .unwrap()
            .accept_report(&admission, wrong, now)
            .is_err()
    );
    let mut wrong = report;
    wrong.state.listeners.get_mut("udp").unwrap().socket_ids = vec!["absent".into()];
    assert!(
        StateHub::new()
            .unwrap()
            .accept_report(&admission, wrong, now)
            .is_err()
    );
}
#[test]
fn peer_projection_expires_candidates_and_does_not_leak_interfaces() {
    let (admission, mut report, mut scope, now) = setup();
    report.state.sockets.get_mut("s1").unwrap().endpoints[0].origin = EndpointSource::Interface {
        interface: "secret-interface".into(),
    };
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    scope.full.clear();
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains("secret-interface"));
    assert!(!json.contains("boundAddress"));
    let view = hub
        .query(
            &scope,
            None,
            &mut GeoIp::default(),
            now + Duration::seconds(11),
        )
        .unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&admission.node_id].report else {
        panic!()
    };
    assert!(report.endpoints.is_empty());
    assert_eq!(
        hub.query(&scope, Some(&[id(999)]), &mut GeoIp::default(), now)
            .unwrap_err(),
        Error::NotFound
    );
}
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

#[test]
fn bounded_journal_replay_detects_gap_without_making_new_samples() {
    let mut hub = StateHub::new().unwrap();
    let network = id(1);
    for _ in 0..600 {
        hub.publish_committed(crate::effect::Effect::Progress {
            network_id: network,
        });
    }
    assert_eq!(
        hub.events.after(0, network).err(),
        Some(Error::InvalidCursor)
    );
    assert_eq!(hub.events.after(599, network).unwrap().len(), 1);
}
pub(super) fn identity(network: NetworkId) -> service::ReadIdentity {
    service::ReadIdentity::UserSession {
        digest: [42; 32],
        network,
    }
}

#[test]
fn permanent_retirement_releases_report_count_and_byte_budget() {
    let (mut admission, report, _, now) = setup();
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    let template = hub.reports[&(admission.network_id, admission.node_id)].clone();
    for n in 2..=MAX_NETWORK_REPORTS {
        hub.reports
            .insert((admission.network_id, id(n as u128)), template.clone());
    }
    hub.bytes = template.bytes * MAX_NETWORK_REPORTS;
    admission.node_id = id(10001);
    assert_eq!(
        hub.accept_report(&admission, report.clone(), now)
            .unwrap_err(),
        Error::StateLimit
    );
    hub.publish_committed(crate::effect::Effect::NodeRetired {
        network_id: admission.network_id,
        node_id: id(1),
    });
    assert_eq!(hub.bytes, template.bytes * (MAX_NETWORK_REPORTS - 1));
    hub.accept_report(&admission, report, now).unwrap();
    assert_eq!(hub.reports.len(), MAX_NETWORK_REPORTS);
}

#[test]
fn installation_resources_are_visible_facts_but_not_peer_candidates() {
    let (mut admission, mut report, mut scope, now) = setup();
    let target = admission.execution.take().unwrap();
    report.state.configuration.applied = None;
    report.state.selection.applied_profile = None;
    let listener = report.state.listeners.get_mut("udp").unwrap();
    listener.phase = ListenerPhase::Preparing;
    listener.configuration = Some(target.clone());
    report.state.sockets.get_mut("s1").unwrap().configuration = Some(target.clone());
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report.clone(), now).unwrap();
    scope.full.clear();
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report: peer }) = &view.nodes[&admission.node_id].report else {
        panic!()
    };
    assert!(peer.endpoints.is_empty());
    report.sequence = Sequence::new(2).unwrap();
    report.state.listeners.get_mut("udp").unwrap().phase = ListenerPhase::Failed;
    hub.accept_report(&admission, report.clone(), now).unwrap();
    report.sequence = Sequence::new(3).unwrap();
    report.state.listeners.get_mut("udp").unwrap().phase = ListenerPhase::Listening;
    assert!(hub.accept_report(&admission, report.clone(), now).is_err());
    report.state.listeners.get_mut("udp").unwrap().phase = ListenerPhase::Preparing;
    report
        .state
        .sockets
        .get_mut("s1")
        .unwrap()
        .configuration
        .as_mut()
        .unwrap()
        .digest = content_digest(b"not the target");
    assert!(hub.accept_report(&admission, report, now).is_err());
}

#[test]
fn retained_configuration_context_does_not_keep_old_peer_endpoints_usable() {
    let (admission, report, mut scope, now) = setup();
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    scope.full.clear();
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&admission.node_id].report else {
        panic!()
    };
    assert_eq!(report.endpoints.len(), 1);
    scope.current.get_mut(&admission.node_id).unwrap().execution = None;
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    let Some(VisibleReport::Peer { report }) = &view.nodes[&admission.node_id].report else {
        panic!()
    };
    assert!(report.endpoints.is_empty());
}

#[test]
fn link_evidence_requires_a_current_execution_not_only_a_fresh_report() {
    let (admission, mut report, mut scope, now) = setup();
    let peer = *scope
        .visible
        .iter()
        .find(|id| **id != admission.node_id)
        .unwrap();
    report.state.links.insert(
        peer,
        vec![LinkState {
            link_id: "link-1".into(),
            transport: "native_udp".into(),
            initiator_node_id: admission.node_id,
            socket_id: "s1".into(),
            phase: LinkPhase::Authenticated,
            business_dependencies: 1,
            observation_ids: vec![],
            observed_at: now,
            expires_at: now + Duration::minutes(1),
            since: Some(now),
        }],
    );
    let mut hub = StateHub::new().unwrap();
    hub.accept_report(&admission, report, now).unwrap();
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    assert_eq!(view.links.len(), 1);
    assert_eq!(view.links[0].evidence, LinkEvidence::OneSided);
    // The instance lease is still valid and the report is young, but the
    // execution lease lapsed: the channel is no longer current evidence.
    scope.current.get_mut(&admission.node_id).unwrap().execution = None;
    let view = hub.query(&scope, None, &mut GeoIp::default(), now).unwrap();
    assert_eq!(view.nodes[&admission.node_id].freshness, Freshness::Current);
    assert_eq!(view.links[0].evidence, LinkEvidence::Expired);
}

#[test]
fn neighbor_probe_uses_explicit_transport_adjacency_not_business_subpath() {
    let (mut admission, _, _, now) = setup();
    let relay = id(90);
    let target = id(2);
    admission.contexts[0].paths = vec![vec![admission.node_id, relay, target]];
    admission.contexts[0].path_deadlines.insert(
        vec![admission.node_id, relay, target],
        now + Duration::minutes(5),
    );
    admission.contexts[0].neighbor_transports =
        BTreeMap::from([(relay, BTreeSet::from(["native_udp".into()]))]);
    let mut probe = ProbeObservation {
        channels: None,
        measurement_id: id(91),
        kind: ProbeKind::Neighbor,
        link_id: Some("real-link".into()),
        transport: Some("native_udp".into()),
        path: vec![admission.node_id, relay],
        publication_id: admission.execution.as_ref().unwrap().execution_publication,
        started_at: now - Duration::milliseconds(1),
        completed_at: Some(now),
        outcome: ProbeOutcome::Success,
        duration_us: Some(Sequence::new(1000).unwrap()),
    };
    validation::validate_probe(&admission, &probe, now).unwrap();
    probe.kind = ProbeKind::Path;
    assert!(validation::validate_probe(&admission, &probe, now).is_err());
    probe.path.push(target);
    probe.transport = None;
    validation::validate_probe(&admission, &probe, now).unwrap();
    probe.kind = ProbeKind::Neighbor;
    probe.transport = Some("native_udp".into());
    probe.path = vec![admission.node_id, target];
    assert!(validation::validate_probe(&admission, &probe, now).is_err());
    probe.path[1] = relay;
    probe.transport = Some("vless".into());
    assert!(validation::validate_probe(&admission, &probe, now).is_err());
}

#[test]
fn wire_report_rejects_nested_positional_objects_and_duplicate_socket_keys() {
    let (_, report, _, _) = setup();
    let mut value = serde_json::to_value(&report).unwrap();
    value["state"]["system"] = serde_json::json!(["host", "linux", "arm64", "test"]);
    assert!(serde_json::from_value::<StateReport>(value).is_err());
    let source = serde_json::to_string(&report).unwrap();
    let socket = serde_json::to_string(&report.state.sockets["s1"]).unwrap();
    let replaced = source.replace(
        &format!("\"sockets\":{{\"s1\":{socket}}}"),
        &format!("\"sockets\":{{\"s1\":{socket},\"s1\":{socket}}}"),
    );
    assert_ne!(source, replaced);
    assert!(serde_json::from_str::<StateReport>(&replaced).is_err());
}
