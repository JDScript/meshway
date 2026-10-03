//! Coherent Mesh source -> compiler -> stable node execution projection.
use super::super::{
    NetworkId, configuration, connections, identity, ipam, managed_transports, tags,
};
use crate::Error;
use meshway_policy::{self as policy, NetworkCompilation, NetworkPolicy};
use meshway_protocol::*;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Publication<'a> {
    db: &'a Connection,
    scope: &'a NetworkId,
    network: MeshNetwork,
    nodes: Vec<MeshNode>,
    adjacency: ConnectionsSnapshot,
    keys: BTreeMap<String, String>,
    compiled: Result<NetworkCompilation, Vec<policy::Diagnostic>>,
}

impl<'a> Publication<'a> {
    pub(in crate::store) fn read(
        db: &'a Connection,
        scope: &'a NetworkId,
        policy: &NetworkPolicy,
    ) -> Result<Self, Error> {
        let network = ipam::network(db, scope)?;
        let nodes = configuration::active_nodes(db, scope)?;
        let adjacency = connections::effective_with_policy(db, scope, policy)?;
        let assignments = tags::assignments(db, scope)?;
        let mut reachable = BTreeSet::new();
        if adjacency.valid {
            for candidate in &adjacency.configuration.candidates {
                let listener = adjacency
                    .configuration
                    .listeners
                    .iter()
                    .find(|l| l.id == candidate.listener_id)
                    .ok_or(Error::Configuration("candidate listener missing"))?;
                reachable.insert(policy::Dial {
                    from: candidate.dialer_node_id.clone(),
                    to: listener.node_id.clone(),
                });
            }
        }
        let facts = policy::Snapshot {
            nodes: nodes
                .iter()
                .map(|n| policy::Node {
                    id: n.id.clone(),
                    tags: assignments.get(&n.id).cloned().unwrap_or_default(),
                    listener: adjacency
                        .configuration
                        .listeners
                        .iter()
                        .any(|l| l.node_id == n.id),
                    exit_enabled: false,
                })
                .collect(),
            tags: tags::catalog(db, scope)?,
            reachable,
            rule_sets: BTreeMap::new(),
        };
        let compiled = policy::compile_network(policy, &facts, policy::CompileOptions { paths: 1 });
        Ok(Self {
            db,
            scope,
            network,
            nodes,
            adjacency,
            keys: identity::bindings(db, scope)?,
            compiled,
        })
    }

    pub(in crate::store) fn valid(&self) -> bool {
        self.compiled.is_ok()
    }

    pub(in crate::store) fn diagnostics(&self) -> Vec<policy::Diagnostic> {
        match &self.compiled {
            Err(d) => d.iter().take(64).cloned().collect(),
            Ok(c) => c
                .base
                .failures
                .iter()
                .take(64)
                .map(|f| policy::Diagnostic {
                    field: format!("mesh.{}", c.base.nodes[usize::from(f.src)]),
                    message: format!(
                        "Candidate path unavailable for {}: {}",
                        f.destination, f.reason
                    ),
                })
                .chain(c.base.mesh_lints.iter().take(64).map(|l| policy::Diagnostic {
                    field: format!("mesh.{}", l.rule_id),
                    message: format!(
                        "{} of {} matching flows {} shadowed (inventory-dependent); example node:{} -> node:{} first matches {} before {}",
                        l.shadowed_flows, l.total_flows,
                        if l.fully_shadowed { "fully" } else { "partially" },
                        c.base.nodes[usize::from(l.example.src)],
                        c.base.nodes[usize::from(l.example.dst)],
                        l.example.winning_rule_id, l.rule_id
                    ),
                }))
                .take(64)
                .collect(),
        }
    }

    fn capable(&self, id: &str) -> Result<bool, Error> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM mesh_execution_capabilities c JOIN connection_runtime_leases l ON c.node_id=l.node_id AND c.network_id=l.network_id AND c.epoch=l.epoch AND c.instance_id=l.instance_id WHERE c.network_id=?1 AND c.node_id=?2 AND c.capability=3)",params![self.scope.as_str(),id],|r|r.get(0))?)
    }

    fn payload(&self, id: &str, key: Option<&[u8]>) -> Result<NodeMeshExecution, Error> {
        let mut nodes = self.nodes.clone();
        let index = nodes
            .iter()
            .position(|n| n.id == id)
            .ok_or(Error::Forbidden)?;
        let node = nodes.remove(index);
        let connections = connections::projection(&self.adjacency, id)?.payload;
        // Applies both to transaction gates (without decrypting) and downloads.
        // Byte budget alone cannot prove the fixed 64-material wire contract.
        if managed_transports::material_count(self.db, self.scope, &connections)?
            > MAX_MESH_EXECUTION_MATERIALS
        {
            return Err(Error::Capacity(
                "Mesh execution exceeds 64 projected materials",
            ));
        }
        let mut paths = Vec::new();
        let mut valid = self.valid() && connections.valid;
        if let Ok(c) = &self.compiled {
            let index = c
                .base
                .nodes
                .iter()
                .position(|n| n == id)
                .ok_or(Error::Configuration("compiled inventory mismatch"))?;
            for path in c.base.exact_authorization(index).grants {
                // Missing capabilities withdraw the complete local execution;
                // never delete unsupported waypoints and replan around them.
                for participant in &path.path {
                    valid &= self.capable(participant)? && self.keys.contains_key(participant);
                }
                paths.push(MeshExecutionPath {
                    nodes: path.path,
                    return_traffic: MeshReturnTraffic::StatefulIpv4V2,
                });
            }
        }
        if paths.len() > MAX_MESH_EXECUTION_PATHS {
            return Err(Error::Capacity("Mesh execution exceeds 128 local paths"));
        }
        let carriers: BTreeSet<Vec<String>> = paths
            .iter()
            .flat_map(|p| [p.nodes.clone(), p.nodes.iter().rev().cloned().collect()])
            .collect();
        if carriers.len() > 128 {
            return Err(Error::Capacity(
                "Mesh execution exceeds 128 forward/return carrier paths",
            ));
        }
        let required: BTreeSet<_> = connections
            .peers
            .iter()
            .map(|p| p.node_id.as_str())
            .chain(
                paths
                    .iter()
                    .flat_map(|p| p.nodes.iter().map(String::as_str)),
            )
            .filter(|p| *p != id)
            .collect();
        if required.len() > MAX_DATAPLANE_IDENTITIES {
            return Err(Error::Capacity(
                "Mesh execution exceeds 64 union identities",
            ));
        }
        // No partial plan is sent when a participant cannot execute or lacks its
        // immutable key. The address binding still prepares a fail-closed sink.
        if !valid {
            paths.clear();
        }
        let required: BTreeSet<_> = connections
            .peers
            .iter()
            .map(|p| p.node_id.as_str())
            .chain(
                paths
                    .iter()
                    .flat_map(|p| p.nodes.iter().map(String::as_str)),
            )
            .filter(|p| *p != id)
            .collect();
        let identities = required
            .into_iter()
            .map(|id| {
                Ok(ConnectionPeer {
                    node_id: id.to_owned(),
                    public_key: self
                        .keys
                        .get(id)
                        .ok_or(Error::Configuration("execution identity missing"))?
                        .clone(),
                })
            })
            .collect::<Result<_, Error>>()?;
        let materials = key
            .map(|key| managed_transports::materials(self.db, self.scope, &connections, key))
            .transpose()?
            .unwrap_or_default();
        if materials.len() > MAX_MESH_EXECUTION_MATERIALS {
            return Err(Error::Capacity(
                "Mesh execution exceeds 64 projected materials",
            ));
        }
        Ok(NodeMeshExecution {
            schema_version: MESH_EXECUTION_VERSION,
            network_id: self.scope.as_str().to_owned(),
            revision: self.adjacency.revision,
            node_id: id.to_owned(),
            required_capability: MeshExecutionCapability::Ipv4StatefulExactPathV3,
            network: self.network.clone(),
            node,
            peers: nodes,
            valid,
            paths,
            identities,
            connections,
            materials,
        })
    }

    pub(in crate::store) fn project(
        &self,
        id: &str,
        key: &[u8],
    ) -> Result<MeshExecutionSnapshot, Error> {
        let payload = self.payload(id, Some(key))?;
        let digest = mesh_execution_digest(&payload)
            .map_err(|_| Error::Configuration("cannot hash Mesh execution"))?;
        let result = MeshExecutionSnapshot { digest, payload };
        if serde_json::to_vec(&result)
            .map_err(|_| Error::Configuration("cannot encode Mesh execution"))?
            .len()
            > MAX_DATAPLANE_BYTES
        {
            return Err(Error::Capacity("Mesh execution exceeds 2 MiB"));
        }
        Ok(result)
    }

    pub(in crate::store) fn ensure_publishable(&self) -> Result<(), Error> {
        for node in &self.nodes {
            let payload = self.payload(&node.id, None)?;
            let bytes = serde_json::to_vec(&payload)
                .map_err(|_| Error::Configuration("cannot budget Mesh execution"))?
                .len();
            if bytes
                + 4096
                + managed_transports::material_budget(self.db, self.scope, &payload.connections)?
                > MAX_DATAPLANE_BYTES
            {
                return Err(Error::Capacity("Mesh execution publication exceeds 2 MiB"));
            }
        }
        Ok(())
    }
}
