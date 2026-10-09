//! Netmap-chain State visibility: what a reader may see follows the current
//! netmaps of the Nodes it owns, never business demands or measurements.
use super::*;
use crate::state::facts::NetmapReader;
use meshway_configuration::netmap::NodeNetmap;

impl Execution {
    /// Netmap chain: what a reader may see is the union of the current
    /// netmap projections of its own Nodes, independent of which business
    /// flows or measurements exist. It changes only with a netmap serial.
    /// Endpoint resources reach a Node reader only for adjacent carriers.
    pub fn netmap_state_scope_in_transaction(
        &self,
        tx: &Transaction<'_>,
        netmaps: &dyn NetmapReader,
        identity: &ReadIdentity,
        now: DateTime<Utc>,
    ) -> Result<VisibilityScope, Error> {
        let ScopeSubject {
            network,
            full,
            expires_at,
            actor,
        } = self.scope_subject(tx, identity, now)?;
        let (revision, cfg, _) = store::current_configuration(tx, &self.sealer, network)?;
        let mut owned = BTreeMap::new();
        for node in &full {
            if let Some(netmap) = netmaps.current(tx, network, *node)? {
                owned.insert(*node, netmap);
            }
        }
        let mut visible = full.clone();
        for netmap in owned.values() {
            visible.extend(crate::state::facts::visible(&netmap.netmap));
        }
        visible.retain(|node| cfg.nodes.contains_key(node));
        let (current, bindings) = self.current_nodes(tx, network, &visible, now)?;
        let endpoint_resources = match identity {
            ReadIdentity::Node { .. } => Some(
                owned
                    .values()
                    .flat_map(|netmap| netmap_endpoint_resources(&netmap.netmap))
                    .collect(),
            ),
            ReadIdentity::UserSession { .. } => None,
        };
        let observation_paths = BTreeSet::new();
        let references: BTreeMap<_, _> = owned
            .iter()
            .map(|(node, n)| (*node, n.reference()))
            .collect();
        let authorization_digest = crate::execution::proof::request_digest(&(
            network,
            revision,
            actor,
            &visible,
            &full,
            &bindings,
            &references,
            &endpoint_resources,
        ))?;
        Ok(VisibilityScope {
            network_id: network,
            configuration_revision: revision,
            authorization_digest,
            observation_authorization_digest: crate::execution::proof::request_digest(&(
                network, revision, actor, &full,
            ))?,
            hostnames: visible
                .iter()
                .filter_map(|node| cfg.nodes.get(node).map(|n| (*node, n.hostname.clone())))
                .collect(),
            visible,
            full,
            endpoint_resources,
            observation_paths,
            expires_at,
            current,
            mesh_addresses: cfg
                .nodes
                .values()
                .flat_map(|n| addresses(&n.addresses))
                .collect(),
        })
    }
}

/// Peer listeners the owner may dial: the only endpoint details it may read,
/// the same rule as its peer facts.
fn netmap_endpoint_resources(netmap: &NodeNetmap) -> BTreeMap<NodeId, BTreeSet<ResourceRef>> {
    let mut resources: BTreeMap<NodeId, BTreeSet<ResourceRef>> = BTreeMap::new();
    for dial in netmap.dialers.values() {
        resources
            .entry(dial.peer)
            .or_default()
            .insert(ResourceRef::Listener {
                key: dial.listener_key.clone(),
            });
    }
    resources
}
