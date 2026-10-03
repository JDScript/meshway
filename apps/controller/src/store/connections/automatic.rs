//! Platform-managed listener advertisements produce ephemeral adjacency intent.
//! The saved advanced connection graph never acquires generated candidates.
use super::{NetworkId, validation};
use crate::Error;
use meshway_policy::{self as policy, NetworkPolicy};
use meshway_protocol::*;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn append(
    db: &Connection,
    network: &NetworkId,
    snapshot: &mut ConnectionsSnapshot,
    policy: &NetworkPolicy,
) -> Result<(), Error> {
    if !snapshot.valid {
        return Ok(());
    }
    let managed = super::super::managed_transports::listeners(db, network)?;
    let assignments = super::super::tags::assignments(db, network)?;
    let nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| n.node.revoked_at.is_none())
        .map(|n| policy::Node {
            id: n.node.id.clone(),
            tags: assignments.get(&n.node.id).cloned().unwrap_or_default(),
            listener: snapshot
                .configuration
                .listeners
                .iter()
                .any(|l| l.node_id == n.node.id),
            exit_enabled: false,
        })
        .collect();
    let streams = streams(db, network)?;
    let mut eligible = Vec::new();
    for summary in &managed {
        if !summary.settings.enabled || !summary.compiled_capability_available {
            continue;
        }
        let Some(listener) = snapshot
            .configuration
            .listeners
            .iter()
            .find(|l| l.id == summary.settings.id)
        else {
            continue;
        };
        let Some(owner) = nodes.iter().find(|n| n.id == listener.node_id) else {
            continue;
        };
        for dialer in &snapshot.nodes {
            let Some(node) = nodes.iter().find(|n| n.id == dialer.node.id) else {
                continue;
            };
            if node.id == owner.id
                || dialer.public_key.is_none()
                || !dialer
                    .capabilities
                    .iter()
                    .any(|c| c.transport == listener.transport && c.can_dial)
                || !policy::connection_allowed(&policy.connections, node, owner)
            {
                continue;
            }
            if let Some(vless) = &summary.settings.vless
                && streams
                    .get(&node.id)
                    .is_none_or(|modes| !modes.contains(&vless.stream))
            {
                continue;
            }
            // An explicit dialer/listener entry overrides automatic priority and
            // endpoint selection for that segment, including its backup list.
            if snapshot
                .configuration
                .candidates
                .iter()
                .any(|c| c.dialer_node_id == node.id && c.listener_id == listener.id)
            {
                continue;
            }
            if !listener.advertised_endpoints.is_empty() {
                eligible.push((listener, dialer.node.id.as_str()));
            }
        }
    }
    if eligible.is_empty() {
        return Ok(());
    }
    let listener_nodes: BTreeMap<_, _> = snapshot
        .configuration
        .listeners
        .iter()
        .map(|l| (l.id.as_str(), l.node_id.as_str()))
        .collect();
    let reachable = snapshot
        .configuration
        .candidates
        .iter()
        .map(|c| policy::Dial {
            from: c.dialer_node_id.clone(),
            to: listener_nodes[c.listener_id.as_str()].to_owned(),
        })
        .chain(eligible.iter().map(|(listener, node)| policy::Dial {
            from: (*node).to_owned(),
            to: listener.node_id.clone(),
        }))
        .collect();
    let facts = policy::Snapshot {
        nodes,
        tags: super::super::tags::catalog(db, network)?,
        reachable,
        rule_sets: BTreeMap::new(),
    };
    // Reuse the authoritative compiler's selected paths for adjacency demand.
    // In particular a relay need not itself be an endpoint in a business grant.
    // Invalid policy never creates opportunistic new connections.
    let Ok(compiled) = policy::compile_network(policy, &facts, policy::CompileOptions { paths: 1 })
    else {
        return Ok(());
    };
    let mut needed = BTreeSet::new();
    for path in &compiled.base.paths {
        for pair in path.windows(2) {
            let a = compiled.base.nodes[usize::from(pair[0])].as_str();
            let b = compiled.base.nodes[usize::from(pair[1])].as_str();
            needed.insert((a.min(b), a.max(b)));
        }
    }
    let mut ids: BTreeSet<_> = snapshot
        .configuration
        .candidates
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let mut generated = Vec::new();
    // Expand endpoints only for compiler-selected adjacency. Inventory is
    // bounded separately; unrelated nodes do not consume the publication budget.
    for (listener, node) in eligible {
        let peer = listener.node_id.as_str();
        if !needed.contains(&(node.min(peer), node.max(peer))) {
            continue;
        }
        for endpoint in &listener.advertised_endpoints {
            if generated.len() + snapshot.configuration.candidates.len() >= 256 {
                return Err(Error::Capacity(
                    "automatic adjacency exceeds 256 candidates",
                ));
            }
            let id = identifier(node, &listener.id, endpoint)?;
            if !ids.insert(id.clone()) {
                return Err(Error::Conflict);
            }
            generated.push(ConnectionCandidate {
                id,
                dialer_node_id: node.to_owned(),
                listener_id: listener.id.clone(),
                endpoint: endpoint.clone(),
                priority: match listener.transport {
                    ConnectionTransport::NativeUdp => 100,
                    ConnectionTransport::NativeTcp => 200,
                    ConnectionTransport::Vless => 300,
                },
                credential_ref: listener.credential_ref.clone(),
            });
        }
    }
    snapshot.configuration.candidates.extend(generated);
    let (normalized, diagnostics) = validation::compile(&snapshot.configuration, &snapshot.nodes);
    if !diagnostics.is_empty() {
        return Err(Error::Configuration(
            "invalid automatic adjacency projection",
        ));
    }
    snapshot.configuration = normalized;
    Ok(())
}

fn streams(
    db: &Connection,
    network: &NetworkId,
) -> Result<BTreeMap<String, Vec<VlessStreamTransport>>, Error> {
    let mut statement = db.prepare(
        "SELECT node_id,vless_streams FROM managed_transport_capabilities WHERE network_id=?1",
    )?;
    statement
        .query_map([network.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|row| {
            let (node, json) = row?;
            let modes = serde_json::from_str(&json)
                .map_err(|_| Error::Configuration("invalid managed stream capabilities"))?;
            Ok((node, modes))
        })
        .collect()
}

fn identifier(node: &str, listener: &str, endpoint: &ConnectionEndpoint) -> Result<String, Error> {
    let bytes = serde_json::to_vec(&(node, listener, endpoint))
        .map_err(|_| Error::Configuration("cannot encode automatic candidate identity"))?;
    // 232 bits retain a bounded identifier with a recognizable resource prefix.
    let digest = Sha256::digest(bytes);
    Ok(format!(
        "auto.{}",
        digest[..29]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}
