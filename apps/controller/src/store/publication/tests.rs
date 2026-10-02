use super::*;
use crate::{NetworkOptions, store::test_support::SingleNetworkFixture};
use std::cell::Cell;

fn node(store: &mut SingleNetworkFixture, seed: u8) -> (String, String) {
    let enrollment = store.create_token(3600).unwrap();
    let credential = local::credential().unwrap();
    let node = store
        .enroll(
            &enrollment.token,
            &EnrollAgentRequest {
                node_credential: credential.clone(),
                name: format!("node-{seed}"),
                agent_version: "test".into(),
            },
        )
        .unwrap();
    store
        .publish_connection_identity(
            &credential,
            &PublishConnectionIdentityRequest {
                public_key: format!("{seed:02x}{}", "00".repeat(31)),
                capabilities: vec![],
            },
        )
        .unwrap();
    (node.id, credential)
}

#[test]
fn one_compilation_is_shared_by_budget_gates_and_multiple_node_snapshots() {
    let dir = crate::store::test_support::directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let nodes: Vec<_> = (9..12).map(|seed| node(&mut store, seed)).collect();
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: store.revision().unwrap(),
            policy: RoutingPolicy {
                default_action: DefaultRouteAction::Direct,
                rules: vec![],
            },
            source_jsonc: None,
        })
        .unwrap();
    compile::COMPILATIONS.with(|count| count.set(0));
    store
        .save_mesh_access(&SaveMeshAccessRequest {
            expected_revision: store.revision().unwrap(),
            policy: MeshAccessPolicy {
                default_action: AccessAction::Allow,
                rules: vec![],
            },
            source_jsonc: None,
        })
        .unwrap();
    compile::COMPILATIONS.with(|count| {
        assert_eq!(
            count.get(),
            1,
            "normal Access write uses one publication compilation"
        )
    });
    let evaluations = Cell::new(0);
    let expected: Vec<_> = nodes
        .iter()
        .map(|(_, credential)| store.agent_dataplane(credential).unwrap())
        .collect();
    let scope = store.network_id.clone();
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .unwrap();
    compile::COMPILATIONS.with(|count| count.set(0));
    ADJACENCY_PROJECTIONS.with(|count| count.set(0));
    let mut publication =
        Publication::read_with(&tx, &scope, || evaluations.set(evaluations.get() + 1)).unwrap();
    publication
        .ensure_publishable(ChangeKind::NetworkOrEnrollment)
        .unwrap();
    for ((id, _), expected) in nodes.iter().zip(expected) {
        let snapshot = publication.dataplane(id).unwrap();
        assert_eq!(
            serde_json::to_vec(&snapshot).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(snapshot.payload.access_policy.grants.len(), 4);
        assert_eq!(snapshot.payload.identities.len(), 2);
        assert_eq!(
            snapshot.payload.revision,
            snapshot.payload.configuration.revision
        );
        assert_eq!(
            snapshot.payload.revision,
            snapshot.payload.connections.revision
        );
    }
    assert_eq!(
        evaluations.get(),
        6,
        "each eligible directed pair evaluated once for all projections"
    );
    compile::COMPILATIONS.with(|count| assert_eq!(count.get(), 1));
    assert_eq!(publication.adjacency.nodes.len(), 3);
    ADJACENCY_PROJECTIONS.with(|count| {
        assert_eq!(
            count.get(),
            3,
            "each node projected once across compilation, budgets and snapshots"
        )
    });
    assert!(publication.adjacency.source.is_some());
}

#[test]
fn routing_repair_keeps_its_existing_dependency_closure() {
    let dir = crate::store::test_support::directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    node(&mut store, 9);
    // Deliberately unreadable unrelated connection source. No executable paths
    // exist under deny-all; routing repair must not acquire an adjacency gate.
    store
        .connection
        .execute(
            "UPDATE connection_source SET configuration='broken' WHERE network_id=?1",
            [store.network_id.as_str()],
        )
        .unwrap();
    let revision = store.revision().unwrap();
    store
        .save_policy(&SavePolicyRequest {
            expected_revision: revision,
            policy: RoutingPolicy {
                default_action: DefaultRouteAction::Direct,
                rules: vec![],
            },
            source_jsonc: None,
        })
        .unwrap();
    assert_eq!(store.revision().unwrap(), revision + 1);
    let scope = store.network_id.clone();
    assert!(matches!(
        finish_change(&store.connection, &scope, ChangeKind::Adjacency),
        Err(Error::Configuration("invalid stored connection plan"))
    ));
}
