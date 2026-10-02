//! Transaction-local publication gates for the current execution contract.
use super::{NetworkId, configuration, connections, network, network_policy};
use crate::Error;
use rusqlite::Connection;

pub(super) mod budget;
pub(super) mod mesh_execution;

#[derive(Clone, Copy)]
pub(super) enum ChangeKind {
    RoutingOrSelectors,
    Adjacency,
    NetworkOrEnrollment,
    NodeWithdrawal,
    MembershipWithdrawal,
}

/// Runs after the revision bump and before commit; a failed gate rolls back the
/// source, credentials, addresses, enrollment consumption and revision together.
pub(super) fn finish_change(
    db: &Connection,
    scope: &NetworkId,
    kind: ChangeKind,
) -> Result<(), Error> {
    let (policy, _) = network_policy::source(db, scope)?;
    mesh_execution::Publication::read(db, scope, &policy)?.ensure_publishable()?;
    let nodes = configuration::active_nodes(db, scope)?;
    if let Some(node) = nodes.first() {
        budget::configuration(configuration::snapshot(db, scope, &node.id)?)?;
    }
    if !matches!(kind, ChangeKind::RoutingOrSelectors) {
        let adjacency = connections::effective_with_policy(db, scope, &policy)?;
        for node in &nodes {
            budget::connections(db, scope, &connections::projection(&adjacency, &node.id)?)?;
        }
    }
    if matches!(
        kind,
        ChangeKind::NetworkOrEnrollment | ChangeKind::MembershipWithdrawal
    ) {
        network::ensure_publishable_with_nodes(db, scope, &nodes)?;
    }
    Ok(())
}
