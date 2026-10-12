use super::*;
use meshway_model::state::{PathEvidence, PathState, ReporterRole};

fn path(nodes: Vec<NodeId>, role: ReporterRole, report: &StateReport) -> PathState {
    PathState {
        nodes,
        netmap: report.state.configuration.applied.unwrap(),
        reporter_role: role,
        evidence: PathEvidence::PeerConfirmed,
        observed_at: report.observed_at,
        expires_at: report.observed_at + Duration::seconds(30),
    }
}

#[test]
fn reported_paths_are_bounded_participating_observations_and_aggregate_while_current() {
    let (mut admission, mut report, mut scope, now) = setup();
    let (node, relay, target) = (admission.node_id, id(2), id(3));
    for context in &mut admission.contexts {
        context.peers.insert(target);
    }
    scope.visible.insert(target);
    scope.hostnames.insert(target, "target".into());
    let mut hub = StateHub::new().unwrap();
    for (nodes, role) in [
        (vec![relay, node, target], ReporterRole::Origin),
        (vec![node, relay, target], ReporterRole::Relay),
        (vec![node, relay, target], ReporterRole::Target),
        (vec![node, relay, id(9)], ReporterRole::Origin),
        (vec![node, relay, node], ReporterRole::Origin),
        (vec![node], ReporterRole::Origin),
    ] {
        report.state.paths = BTreeMap::from([("c".into(), path(nodes, role, &report))]);
        assert_eq!(
            hub.accept_report(&admission, report.clone(), now)
                .unwrap_err(),
            Error::InvalidRequest
        );
    }
    report.state.paths = (0..=meshway_protocol::STATE_MAX_PATHS)
        .map(|i| {
            (
                format!("c{i}"),
                path(vec![node, relay, target], ReporterRole::Origin, &report),
            )
        })
        .collect();
    assert_eq!(
        hub.accept_report(&admission, report.clone(), now)
            .unwrap_err(),
        Error::StateLimit
    );
    report.state.paths = BTreeMap::from([(
        "c".into(),
        path(vec![node, relay, target], ReporterRole::Origin, &report),
    )]);
    hub.accept_report(&admission, report.clone(), now).unwrap();
    let view = hub.query(&scope, None, now).unwrap();
    assert_eq!(view.paths.len(), 1);
    assert_eq!(view.paths[0].state.nodes, [node, relay, target]);
    assert_eq!(view.paths[0].reporter.node_id, node);
    // A path through a Node the caller does not see is withheld.
    scope.visible.remove(&target);
    scope.hostnames.remove(&target);
    assert!(hub.query(&scope, None, now).unwrap().paths.is_empty());
    scope.visible.insert(target);
    scope.hostnames.insert(target, "target".into());
    // An expired observation is not a current path.
    assert!(
        hub.query(&scope, None, now + Duration::seconds(31))
            .unwrap()
            .paths
            .is_empty()
    );
}
