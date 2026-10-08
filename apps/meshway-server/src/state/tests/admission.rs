use super::*;

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
            acquisition: None,
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
