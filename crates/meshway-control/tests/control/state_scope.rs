//! Genuine Node sessions/configurations/receipts; resource facts are protocol
//! fixture observations, not evidence of OS installation or packet delivery.
use super::{
    faults::{Fault, Proxy},
    refresh::{control, demand, listeners},
    support::{Owner, Server},
    system,
};
use meshway_control::{Control, Error};
use meshway_control_client::GetNodeStatePath;
use meshway_model::{NodeId, Sequence, state::*};
use meshway_protocol::state::{NodeStateView, VisibleReport};
use std::collections::BTreeMap;

fn facts(node: &Control) -> NodeState {
    let target = node.cached_target().unwrap();
    let reference = node.status().cached.unwrap();
    let peer = *target.configuration.peers.keys().next().unwrap();
    let now = chrono::Utc::now();
    NodeState {
        system: system(),
        capabilities: target.configuration.self_keys.capabilities.clone(),
        configuration: ConfigurationState {
            saved: Some(reference.clone()),
            applied: Some(reference.clone()),
            progress: None,
        },
        listeners: BTreeMap::from([(
            "udp".into(),
            ListenerState {
                phase: ListenerPhase::Listening,
                socket_ids: vec!["runtime-udp".into()],
                loaded_certificate_sha256: None,
                problem: None,
                configuration: Some(reference.clone()),
            },
        )]),
        sockets: BTreeMap::from([(
            "runtime-udp".into(),
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
                link_id: "observed-link".into(),
                transport: "native_udp".into(),
                initiator_node_id: peer,
                socket_id: "runtime-udp".into(),
                phase: LinkPhase::Authenticated,
                business_dependencies: 1,
                observation_ids: vec![],
                observed_at: now,
                expires_at: now + chrono::Duration::seconds(30),
                since: Some(now),
            }],
        )]),
        paths: BTreeMap::new(),
        selection: SelectionState {
            applied_profile: Some("mesh".into()),
        },
        probes: vec![],
        problems: vec![],
    }
}

async fn installed(node: &mut Control) {
    node.refresh_configuration().await.unwrap();
    let ticket = node.begin_application().await.unwrap();
    node.finish_application(&ticket, Ok(()), chrono::Utc::now())
        .await
        .unwrap();
}

async fn view(server: &Server, owner: &Owner, node: NodeId) -> NodeStateView {
    server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: owner.network,
                node_id: node,
            },
            owner.token.expose(),
        )
        .await
        .unwrap()
        .nodes
        .remove(&node)
        .unwrap()
}

#[tokio::test]
async fn known_report_scope_race_preserves_authority_and_requires_a_fresh_higher_sequence() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut a = control(&server, &owner, &server.origin, "scope-a").await;
    let mut b = control(&server, &owner, &proxy.origin, "scope-b").await;
    listeners(&server, &owner).await;
    let first_demand = demand(&a, &b, 443);
    a.create_demand(&first_demand).await.unwrap();
    installed(&mut b).await;
    let initial_facts = facts(&b);
    let accepted = b
        .report_runtime_state(initial_facts.clone(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(accepted.sequence, Sequence::new(1).unwrap());
    let node_id = b.status().binding.unwrap().node_id;
    let old = b.execution_snapshot().unwrap();
    let lease = b.status().instance.unwrap();
    let sampled = facts(&b);
    let next_demand = demand(&a, &b, 5353);
    a.create_demand(&next_demand).await.unwrap();
    let before = view(&server, &owner, node_id).await;
    assert!(matches!(
        b.report_runtime_state(sampled, chrono::Utc::now()).await,
        Err(Error::StateScopeChanged)
    ));
    assert_eq!(b.status().instance.unwrap(), lease);
    old.ensure_current().unwrap();
    assert!(b.status().authenticated);
    assert!(matches!(
        b.retry_state_report().await,
        Err(Error::Lifecycle)
    ));
    let after = view(&server, &owner, node_id).await;
    assert_eq!(after.report_sequence, before.report_sequence);
    assert_eq!(after.received_at, before.received_at);
    assert_eq!(after.observed_at, before.observed_at);
    assert_eq!(after.last_authenticated_at, before.last_authenticated_at);
    assert_eq!(after.report, before.report);
    installed(&mut b).await;
    let current = b.execution_snapshot().unwrap();
    current.continuation_from(&old).unwrap();
    assert!(old.ensure_current().is_err());
    let fresh = facts(&b);
    let accepted = b
        .report_runtime_state(fresh.clone(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(
        accepted.sequence.get(),
        3,
        "rejected sequence 2 cannot be reused"
    );
    let latest = view(&server, &owner, node_id).await;
    assert_eq!(latest.report_sequence, Some(accepted.sequence));
    assert_eq!(
        latest.report,
        Some(VisibleReport::Full {
            report: Box::new(fresh)
        })
    );

    // An already accepted sample with lost delivery remains in Server history.
    // A later typed scope rejection discards only the pending local retry.
    proxy.fault("/agent-api/v1/state", Fault::Lost);
    assert!(
        b.report_runtime_state(facts(&b), chrono::Utc::now())
            .await
            .is_err()
    );
    let recorded = view(&server, &owner, node_id).await;
    let another = demand(&a, &b, 8443);
    a.create_demand(&another).await.unwrap();
    assert!(matches!(
        b.retry_state_report().await,
        Err(Error::StateScopeChanged)
    ));
    let retained = view(&server, &owner, node_id).await;
    assert_eq!(retained.report_sequence, recorded.report_sequence);
    assert_eq!(retained.received_at, recorded.received_at);
    installed(&mut b).await;
    let accepted = b
        .report_runtime_state(facts(&b), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(accepted.sequence.get(), 5);
    b.shutdown().await.unwrap();
    a.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn only_exact_state_endpoint_scope_problem_avoids_generic_conflict_fencing() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    for (index, fault, endpoint) in [
        (0, Fault::UntypedConflict, "/agent-api/v1/state"),
        (1, Fault::WrongStateScopeStatus, "/agent-api/v1/state"),
        (2, Fault::StateScopeChanged, "/configuration/refresh"),
    ] {
        let mut node = control(
            &server,
            &owner,
            &proxy.origin,
            &format!("bad-scope-{index}"),
        )
        .await;
        node.refresh_configuration().await.unwrap();
        node.acknowledge_saved().await.unwrap();
        let old = node.execution_snapshot().unwrap();
        proxy.fault(endpoint, fault);
        let error = if endpoint == "/configuration/refresh" {
            node.refresh_configuration().await.unwrap_err()
        } else {
            node.report_control_state(system()).await.unwrap_err()
        };
        assert!(!matches!(error, Error::StateScopeChanged));
        assert!(old.ensure_current().is_err());
        assert!(node.status().instance.is_none());
        assert!(!node.status().authenticated);
        node.shutdown().await.unwrap();
    }
    proxy.close().await;
    server.close().await;
}
