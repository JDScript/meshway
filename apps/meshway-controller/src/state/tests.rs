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
        lease_expires_at: now + Duration::minutes(5),
        execution: Some(reference.clone()),
        configurations: vec![reference.clone()],
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
        profiles: BTreeSet::from(["mesh".into()]),
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
        visible: BTreeSet::from([node, peer]),
        full: BTreeSet::from([node]),
        observation_paths: BTreeSet::from([vec![node, peer]]),
        mesh_addresses: admission.mesh_addresses.clone(),
        expires_at: now + Duration::minutes(10),
        current: BTreeMap::from([(
            node,
            CurrentNode {
                instance_id: Some(admission.instance_id),
                configurations: vec![reference],
                revoked: false,
                lease_expires_at: Some(admission.lease_expires_at),
            },
        )]),
    };
    (admission, report, scope, now)
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
        StateError::Conflict
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
        StateError::Unauthorized
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
        StateError::NotFound
    );
}
#[test]
fn observations_bind_instance_path_and_independent_subscriber_leases() {
    let (report, _, scope, now) = setup();
    let mut admission = TaskAdmission {
        report: report.clone(),
        session_id: id(10),
        authorization_digest: scope.authorization_digest,
        session_expires_at: now + Duration::minutes(10),
    };
    let request = CreateObservation {
        source_node_id: report.node_id,
        target_node_id: id(2),
        selection: ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    let mut observations = Observations::default();
    let first = observations
        .create(&admission, request.clone(), now)
        .unwrap();
    assert!(first.latest.is_none());
    assert!(first.path.is_none());
    admission.session_id = id(11);
    let second = observations.create(&admission, request, now).unwrap();
    assert_eq!(first.id, second.id);
    observations
        .cancel(id(10), report.network_id, first.id)
        .unwrap();
    assert_eq!(observations.node_tasks(&report, now).len(), 1);
    assert!(
        observations
            .node_tasks(&report, now + Duration::seconds(31))
            .is_empty()
    );
    assert!(
        observations
            .renew(&admission, first.id, now + Duration::seconds(31))
            .is_err()
    );
}

#[test]
fn shared_task_subscribers_renew_independently_and_create_retry_does_not_renew() {
    let (report, _, scope, now) = setup();
    let mut admission = TaskAdmission {
        report: report.clone(),
        session_id: id(10),
        authorization_digest: scope.authorization_digest,
        session_expires_at: now + Duration::minutes(5),
    };
    let request = CreateObservation {
        source_node_id: report.node_id,
        target_node_id: id(2),
        selection: ObservationPath::Auto,
        metrics: vec![meshway_model::observations::MetricKind::Rtt],
        interval_ms: 1000,
    };
    let mut tasks = Observations::default();
    let first = tasks.create(&admission, request.clone(), now).unwrap();
    let retry = tasks
        .create(&admission, request.clone(), now + Duration::seconds(6))
        .unwrap();
    assert_eq!(retry.subscriber_expires_at, first.subscriber_expires_at);
    admission.session_id = id(11);
    let second = tasks
        .create(&admission, request, now + Duration::seconds(5))
        .unwrap();
    assert_eq!(first.id, second.id);
    admission.session_id = id(10);
    tasks
        .renew(&admission, first.id, now + Duration::seconds(10))
        .unwrap();
    admission.session_id = id(11);
    let renewed = tasks
        .renew(&admission, second.id, now + Duration::seconds(15))
        .unwrap();
    assert_eq!(
        renewed.subscriber_expires_at,
        Some(now + Duration::seconds(45))
    );
    tasks.cancel(id(11), report.network_id, first.id).unwrap();
    assert_eq!(
        tasks.node_tasks(&report, now + Duration::seconds(16))[0].expires_at,
        now + Duration::seconds(40)
    );
}

#[test]
fn bounded_journal_replay_detects_gap_without_making_new_samples() {
    let mut hub = StateHub::new().unwrap();
    let network = id(1);
    for _ in 0..600 {
        hub.publish_committed(stream::CommittedChange::Progress {
            network_id: network,
        });
    }
    assert_eq!(hub.events.after(0, network).err(), Some(StateError::Cursor));
    assert_eq!(hub.events.after(599, network).unwrap().len(), 1);
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
