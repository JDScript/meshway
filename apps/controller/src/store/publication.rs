//! Controller-only publication boundary. A context is scoped to one locked Store
//! call / SQLite transaction, never cached across requests or revisions.
use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::*;
use rusqlite::Connection;

use super::{NetworkId, access, configuration, connections, ipam, network, tags};
use crate::Error;

pub(super) mod budget;
mod compile;
mod project;
#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static ADJACENCY_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Existing mutation dependency closures. These are budget gates, not a new
/// requirement that saved sources remain valid after withdrawal or tag removal.
#[derive(Clone, Copy)]
pub(super) enum ChangeKind {
    RoutingOrSelectors,
    Adjacency,
    NetworkOrEnrollment,
    NodeWithdrawal,
    MembershipWithdrawal,
}

pub(super) struct RoutingFacts {
    revision: u32,
    network: MeshNetwork,
    nodes: Vec<MeshNode>,
    policy: RoutingPolicy,
}

impl RoutingFacts {
    pub(super) fn read(db: &Connection, scope: &NetworkId) -> Result<Self, Error> {
        Ok(Self {
            revision: configuration::revision(db, scope)?,
            network: ipam::network(db, scope)?,
            nodes: configuration::active_nodes(db, scope)?,
            policy: configuration::source_policy(db, scope)?,
        })
    }

    pub(super) fn configuration(&self, id: &str) -> Result<ConfigurationSnapshot, Error> {
        let prepared = compile::prepare(&self.policy, &self.network, &self.nodes);
        project::configuration(self, prepared.report(), id)
    }
}

/// Only public facts: node identities are public keys, never material secrets.
struct Facts {
    routing: RoutingFacts,
    access: MeshAccessPolicy,
    catalog: BTreeSet<String>,
    assignments: BTreeMap<String, BTreeSet<String>>,
    keys: BTreeMap<String, String>,
}

impl Facts {
    fn read(db: &Connection, scope: &NetworkId) -> Result<Self, Error> {
        Ok(Self {
            routing: RoutingFacts::read(db, scope)?,
            access: access::source(db, scope)?,
            catalog: tags::catalog(db, scope)?,
            assignments: tags::assignments(db, scope)?,
            keys: access::bindings(db, scope)?,
        })
    }
}

/// The concrete compiler boundary: database facts -> policy internal report and
/// exact paths -> protocol node DTOs. A future v3 adapter belongs here, not in
/// protocol or in the Store mutation handlers.
pub(super) struct Publication<'a> {
    facts: Facts,
    routing: meshway_policy::CompileReport,
    access_valid: bool,
    authorizations: BTreeMap<String, MeshPathAuthorization>,
    adjacency: Adjacency<'a>,
}

/// Lazy because route/selector-only repair does not require a connection budget
/// gate when no authorized path is executable. Reuses each public projection.
struct Adjacency<'a> {
    db: &'a Connection,
    scope: &'a NetworkId,
    source: Option<ConnectionsSnapshot>,
    nodes: BTreeMap<String, AgentConnectionsSnapshot>,
}

impl Adjacency<'_> {
    fn project(&mut self, id: &str) -> Result<&AgentConnectionsSnapshot, Error> {
        if !self.nodes.contains_key(id) {
            if self.source.is_none() {
                self.source = Some(connections::admin_snapshot(self.db, self.scope)?);
            }
            #[cfg(test)]
            ADJACENCY_PROJECTIONS.with(|count| count.set(count.get() + 1));
            let snapshot =
                connections::projection(self.source.as_ref().expect("loaded connections"), id)?;
            self.nodes.insert(id.to_owned(), snapshot);
        }
        Ok(self.nodes.get(id).expect("projected adjacency"))
    }
}

impl<'a> Publication<'a> {
    pub(super) fn read(db: &'a Connection, scope: &'a NetworkId) -> Result<Self, Error> {
        Self::read_with(db, scope, || {})
    }

    pub(super) fn read_with(
        db: &'a Connection,
        scope: &'a NetworkId,
        route_evaluated: impl FnMut(),
    ) -> Result<Self, Error> {
        let facts = Facts::read(db, scope)?;
        let prepared = compile::prepare(
            &facts.routing.policy,
            &facts.routing.network,
            &facts.routing.nodes,
        );
        let routing = prepared.report().clone();
        let mut adjacency = Adjacency {
            db,
            scope,
            source: None,
            nodes: BTreeMap::new(),
        };
        let (access_valid, authorizations) =
            compile::authorizations(&facts, &prepared, &mut adjacency, route_evaluated)?;
        Ok(Self {
            facts,
            routing,
            access_valid,
            authorizations,
            adjacency,
        })
    }

    pub(super) fn into_authorizations(self) -> (bool, BTreeMap<String, MeshPathAuthorization>) {
        (self.access_valid, self.authorizations)
    }

    pub(super) fn dataplane(&mut self, id: &str) -> Result<AgentDataplaneSnapshot, Error> {
        let configuration = project::configuration(&self.facts.routing, &self.routing, id)?.payload;
        let connections = self.adjacency.project(id)?.payload.clone();
        let access_policy = self.authorizations.get(id).cloned().unwrap_or_default();
        let identities = project::identities(id, &connections, &access_policy, &self.facts.keys)?;
        if connections.revision != configuration.revision {
            return Err(Error::Configuration("incoherent dataplane revision"));
        }
        let payload = NodeDataplane {
            schema_version: DATAPLANE_SCHEMA_VERSION,
            revision: configuration.revision,
            node_id: id.to_owned(),
            configuration,
            connections,
            access_policy,
            access_valid: self.access_valid,
            identities,
        };
        let digest = dataplane_digest(&payload)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?;
        let snapshot = AgentDataplaneSnapshot { digest, payload };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?
            .len()
            > MAX_DATAPLANE_BYTES
        {
            return Err(Error::Capacity(
                "dataplane exceeds 2 MiB publication budget",
            ));
        }
        Ok(snapshot)
    }

    fn ensure_publishable(&mut self, kind: ChangeKind) -> Result<(), Error> {
        // The same active inventory occurs exactly once as self + peers; moving
        // a node between these positions preserves size, so one config suffices.
        if let Some(node) = self.facts.routing.nodes.first() {
            let snapshot = project::configuration(&self.facts.routing, &self.routing, &node.id)?;
            budget::configuration(snapshot)?;
        }
        if !matches!(kind, ChangeKind::RoutingOrSelectors) {
            let ids: Vec<_> = self
                .facts
                .routing
                .nodes
                .iter()
                .map(|n| n.id.clone())
                .collect();
            for id in ids {
                let db = self.adjacency.db;
                let scope = self.adjacency.scope;
                let projected = self.adjacency.project(&id)?;
                // Private material stays in managed_transports. Only its byte
                // reservation crosses this boundary, preserving the old gate.
                budget::connections(db, scope, projected)?;
            }
        }
        if matches!(
            kind,
            ChangeKind::NetworkOrEnrollment | ChangeKind::MembershipWithdrawal
        ) {
            network::ensure_publishable_with_nodes(
                self.adjacency.db,
                self.adjacency.scope,
                &self.facts.routing.nodes,
            )?;
        }
        Ok(())
    }
}

/// Call after the existing revision bump, before commit. Any failure rolls back
/// the source, enrollment consumption, addresses, identities and revision.
pub(super) fn finish_change(
    db: &Connection,
    scope: &NetworkId,
    kind: ChangeKind,
) -> Result<(), Error> {
    Publication::read(db, scope)?.ensure_publishable(kind)
}
