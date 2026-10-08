//! Real enrollment/publications/receipts back these resource-fact fixtures.
//! Socket/link metadata validates reporting; it does not claim packet I/O.
use super::node::{caps, fixture, session};
use crate::{accounts::random_uuid, error::Error, state::service::ReadIdentity, store};
use meshway_configuration::execution::{BusinessProtocol, BusinessTarget, CompiledNode};
use meshway_model::{InstanceId, RequestId, Sequence, Version, state::*};
use meshway_protocol::{execution::*, state::StateReport};
use std::collections::BTreeMap;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn report(target: &CompiledNode, instance_id: InstanceId, sequence: u64) -> StateReport {
    let now = chrono::Utc::now();
    let cfg = &target.configuration;
    let reference = ConfigurationRef {
        revision: cfg.revision,
        execution_publication: cfg.publication,
        digest: target.digest,
    };
    let peer = *cfg.peers.keys().next().unwrap();
    StateReport {
        instance_id,
        sequence: Sequence::new(sequence).unwrap(),
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "fixture".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "fixture".into(),
            },
            capabilities: cfg.self_keys.capabilities.clone(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference.clone()),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["socket".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    configuration: Some(reference.clone()),
                },
            )]),
            sockets: BTreeMap::from([(
                "socket".into(),
                SocketState {
                    local_endpoints: None,
                    resource: ResourceRef::Listener { key: "udp".into() },
                    configuration: Some(reference),
                    generation: Sequence::new(1).unwrap(),
                    network: SocketNetwork::Udp,
                    bound_address: "127.0.0.1:23456".parse().unwrap(),
                    endpoints: vec![],
                },
            )]),
            links: BTreeMap::from([(
                peer,
                vec![LinkState {
                    acquisition: None,
                    link_id: "fixture-link".into(),
                    transport: "native_udp".into(),
                    initiator_node_id: peer,
                    socket_id: "socket".into(),
                    phase: LinkPhase::Authenticated,
                    business_dependencies: 1,
                    observation_ids: vec![],
                    observed_at: now,
                    expires_at: now + chrono::Duration::seconds(30),
                    since: Some(now),
                }],
            )]),
            paths: BTreeMap::from([(
                "observed-path".into(),
                PathState {
                    nodes: cfg.permits.values().next().unwrap().paths[0].nodes.clone(),
                    publication_id: cfg.publication,
                    reporter_role: ReporterRole::Target,
                    evidence: PathEvidence::Selected,
                    observed_at: now,
                    expires_at: now + chrono::Duration::seconds(30),
                },
            )]),
            selection: SelectionState {
                applied_profile: Some(cfg.dns.profile.clone()),
            },
            probes: vec![],
            problems: vec![],
        },
    }
}

#[tokio::test]
async fn known_stale_scope_is_rejected_without_history_or_sequence_and_bad_facts_stay_invalid() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let s = app.execution();
    let at = session(&s, network, a, &ak, &signer_a).await;
    let bt = session(&s, network, b, &bk, &signer_b).await;
    let acquire = || AcquireInstanceRequest {
        instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
        capabilities: caps(),
    };
    let ai = s.acquire(at.token.expose(), acquire()).await.unwrap();
    let bi = s.acquire(bt.token.expose(), acquire()).await.unwrap();
    let ar = InstanceRequest {
        instance_id: ai.instance_id,
        generation: ai.generation,
    };
    let br = InstanceRequest {
        instance_id: bi.instance_id,
        generation: bi.generation,
    };
    let demand = |port| DemandRequest {
        instance: ar.clone(),
        request_id: rid(),
        target: BusinessTarget::Mesh { node: b },
        protocol: BusinessProtocol::Udp,
        port: Some(meshway_configuration::transport::Port::new(port).unwrap()),
        profile: "mesh".into(),
        duration_seconds: 60,
    };
    s.demand(at.token.expose(), demand(53)).await.unwrap();
    let query = || NodeConfigurationRequest {
        instance: br.clone(),
        base: None,
    };
    let first = s
        .refresh_configuration(bt.token.expose(), query())
        .await
        .unwrap();
    let old = first.full.unwrap();
    for (sequence, phase) in [(1, ReceiptPhase::Saved), (2, ReceiptPhase::Applied)] {
        s.receipt(
            bt.token.expose(),
            ReceiptRequest {
                instance: br.clone(),
                target: first.lease.target.clone().unwrap(),
                phase,
                attempt: Version::MIN,
                sequence: Sequence::new(sequence).unwrap(),
                reported_at_ms: store::execution::now().unwrap() as u64,
                failure: None,
            },
        )
        .await
        .unwrap();
    }
    let state = app.node_state_service().unwrap();
    let first_report = report(&old, bi.instance_id, 1);
    state
        .accept_report(bt.token.expose().to_owned(), first_report.clone())
        .await
        .unwrap();
    s.demand(at.token.expose(), demand(5353)).await.unwrap();
    let identity = || ReadIdentity::Node {
        authorization: bt.token.expose().to_owned(),
    };
    let before = state.query(identity(), None).await.unwrap();
    let events = app.state_runtime.lock().unwrap().hub.events.sequence();
    let stale = report(&old, bi.instance_id, 2);
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), stale.clone())
            .await
            .unwrap_err(),
        Error::StateScopeChanged
    );
    for keep_listener in [true, false] {
        let mut resources_only = stale.clone();
        resources_only.state.links.clear();
        resources_only.state.paths.clear();
        resources_only.state.selection.applied_profile = None;
        if !keep_listener {
            resources_only.state.listeners.clear();
        }
        assert_eq!(
            state
                .accept_report(bt.token.expose().to_owned(), resources_only)
                .await
                .unwrap_err(),
            Error::StateScopeChanged
        );
    }
    let mut malformed_path = stale.clone();
    malformed_path
        .state
        .paths
        .get_mut("observed-path")
        .unwrap()
        .reporter_role = ReporterRole::Origin;
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), malformed_path)
            .await
            .unwrap_err(),
        Error::InvalidRequest
    );
    // An already accepted duplicate can be rejected in the new scope without
    // changing the sample retained from its original acceptance.
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), first_report)
            .await
            .unwrap_err(),
        Error::StateScopeChanged
    );
    let mut foreign = stale.clone();
    foreign.state.configuration.saved = ai.target;
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), foreign)
            .await
            .unwrap_err(),
        Error::InvalidRequest
    );
    let mut malformed = stale.clone();
    malformed
        .state
        .sockets
        .get_mut("socket")
        .unwrap()
        .bound_address
        .set_port(0);
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), malformed)
            .await
            .unwrap_err(),
        Error::InvalidRequest
    );
    let mut unknown = stale;
    unknown.state.sockets.get_mut("socket").unwrap().resource = ResourceRef::Listener {
        key: "unissued".into(),
    };
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), unknown)
            .await
            .unwrap_err(),
        Error::InvalidRequest
    );
    assert_eq!(
        app.state_runtime.lock().unwrap().hub.events.sequence(),
        events
    );
    let after = state.query(identity(), None).await.unwrap();
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.nodes[&b], before.nodes[&b]);
    let current = s
        .refresh_configuration(bt.token.expose(), query())
        .await
        .unwrap()
        .full
        .unwrap();
    let fresh = report(&current, bi.instance_id, 3);
    let mut malformed = fresh.clone();
    malformed
        .state
        .sockets
        .get_mut("socket")
        .unwrap()
        .bound_address
        .set_port(0);
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), malformed)
            .await
            .unwrap_err(),
        Error::InvalidRequest
    );
    // A current known target whose execution lease was withdrawn is likewise a
    // report-scope rejection, not a credential failure or implicit lease grant.
    app.executor.handle().execute(move |db| {
        db.execute("UPDATE node_execution SET execution_expires_at=NULL WHERE network_id=?1 AND node_id=?2", rusqlite::params![network.as_uuid().as_bytes().as_slice(), b.as_uuid().as_bytes().as_slice()]).unwrap();
    }).await.unwrap();
    assert_eq!(
        state
            .accept_report(bt.token.expose().to_owned(), fresh.clone())
            .await
            .unwrap_err(),
        Error::StateScopeChanged
    );
    let no_grant = s.configuration(bt.token.expose(), query()).await.unwrap();
    assert_eq!(no_grant.lease.execution_expires_at_ms, None);
    s.refresh_configuration(bt.token.expose(), query())
        .await
        .unwrap();
    let accepted = state
        .accept_report(bt.token.expose().to_owned(), fresh)
        .await
        .unwrap();
    assert_eq!(accepted.sequence.get(), 3);
    app.shutdown().await.unwrap();
}
