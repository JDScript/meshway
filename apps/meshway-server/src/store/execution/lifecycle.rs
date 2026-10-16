//! Node lifecycle effects of configuration changes: session retirement,
//! revocation of removed Nodes, the compilation inputs every netmap shares
//! and the netmap targets of a version.
use super::{current_configuration, id, json, subjects, user_binding};
use crate::configuration::ConfigurationBindings;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_configuration::{NetworkConfiguration, execution::TrustedNode};
use meshway_model::{
    BoundNodeKey, Digest, KeyAlgorithm, NetmapRef, NetworkId, NodeId, NodePublicKey, Version,
};
use rusqlite::{Transaction, params};
use std::collections::BTreeMap;

/// Who the configuration's subjects are, which nodes hold trusted keys, and
/// which profile each node selected under its current identity: the inputs
/// compilation and planning share for one configuration and its bindings.
pub(crate) struct CompilationContext {
    pub(crate) subjects: meshway_policy::SubjectContext,
    pub(crate) inventory: BTreeMap<NodeId, TrustedNode>,
    pub(crate) selected_profiles: BTreeMap<NodeId, String>,
}
pub(crate) fn compilation_context(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &ConfigurationBindings,
) -> Result<CompilationContext, Error> {
    let selected_profiles = super::selection::selections(tx, network, cfg, bindings)?
        .into_iter()
        .filter_map(|(node, stored)| Some((node, stored.profile?)))
        .collect();
    let subjects = subjects(tx, network, bindings)?;
    let inventory = trusted_inventory(tx, network, cfg, &subjects)?;
    Ok(CompilationContext {
        subjects,
        inventory,
        selected_profiles,
    })
}
fn trusted_inventory(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    subjects: &meshway_policy::SubjectContext,
) -> Result<BTreeMap<NodeId, TrustedNode>, Error> {
    let mut out = BTreeMap::new();
    let mut stmt=tx.prepare("SELECT n.id,k.id,k.public_key,k.generation,d.id,d.public_key,d.generation,e.capabilities FROM nodes n JOIN node_keys k ON k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL JOIN node_keys d ON d.network_id=n.network_id AND d.node_id=n.id AND d.purpose='dataplane' AND d.retired_at IS NULL AND d.revoked_at IS NULL LEFT JOIN node_execution e ON e.network_id=n.network_id AND e.node_id=n.id WHERE n.network_id=?1 AND n.credentials_revoked_at IS NULL")?;
    let mut rows = stmt.query([network.as_uuid().as_bytes().as_slice()])?;
    while let Some(r) = rows.next()? {
        let node = id(r.get(0)?)?;
        if !subjects.node_is_active(cfg, node) {
            continue;
        }
        let key = |offset| -> Result<BoundNodeKey, Error> {
            Ok(BoundNodeKey {
                id: id(r.get(offset)?)?,
                generation: Version::new(
                    r.get::<_, i64>(offset + 2)?
                        .try_into()
                        .map_err(|_| Error::decode("node_keys.generation"))?,
                )
                .map_err(|_| Error::decode("node_keys.generation"))?,
                public_key: NodePublicKey {
                    algorithm: if offset == 1 {
                        KeyAlgorithm::Ed25519
                    } else {
                        KeyAlgorithm::X25519
                    },
                    bytes: Digest::from_bytes(
                        r.get::<_, Vec<u8>>(offset + 1)?
                            .try_into()
                            .map_err(|_| Error::decode("node_keys.public_key"))?,
                    ),
                },
            })
        };
        let capabilities = r
            .get::<_, Option<Vec<u8>>>(7)?
            .map(|v| json(&v))
            .transpose()?
            .unwrap_or_default();
        out.insert(
            node,
            TrustedNode {
                control_key: key(1)?,
                dataplane_key: key(4)?,
                capabilities,
            },
        );
    }
    Ok(out)
}

/// Ends every live control session and instance of one Node. The Node must
/// authenticate again and acquire a new instance; its keys and netmap are
/// untouched.
pub(crate) fn retire_node_sessions(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM node_sessions WHERE network_id=?1 AND node_id=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    tx.execute(
        "UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3",
        params![
            now,
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok(())
}
/// Config owner invokes within its source commit transaction after writing the
/// authoritative configuration and before recompiling netmaps. Removed nodes
/// are permanent revocations, including JSONC deletion; identity changes end
/// sessions; removed addresses are retired.
pub(crate) fn apply_configuration_effects(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    previous: &NetworkConfiguration,
    previous_bindings: &ConfigurationBindings,
    target: &NetworkConfiguration,
    now: i64,
) -> Result<(), Error> {
    let (revision, committed, bindings) = current_configuration(tx, sealer, network)?;
    if &committed != target {
        return Err(Error::Internal(
            "committed configuration differs from the prepared target",
        ));
    }
    for (node, old) in &previous.nodes {
        if target
            .nodes
            .get(node)
            .is_some_and(|new| match (&old.identity, &new.identity) {
                (
                    meshway_configuration::NodeIdentity::User { .. },
                    meshway_configuration::NodeIdentity::User { .. },
                ) => {
                    user_binding(previous, previous_bindings, *node)
                        != user_binding(target, &bindings, *node)
                }
                _ => new.identity != old.identity,
            })
        {
            retire_node_sessions(tx, network, *node, now)?;
        }
    }
    for (node, old) in &previous.nodes {
        let addresses = [
            old.addresses.ipv4.map(std::net::IpAddr::V4),
            old.addresses.ipv6.map(std::net::IpAddr::V6),
        ];
        let new = target
            .nodes
            .get(node)
            .map(|n| {
                [
                    n.addresses.ipv4.map(std::net::IpAddr::V4),
                    n.addresses.ipv6.map(std::net::IpAddr::V6),
                ]
            })
            .unwrap_or([None, None]);
        for address in addresses
            .into_iter()
            .flatten()
            .filter(|a| !new.contains(&Some(*a)))
        {
            tx.execute("INSERT INTO address_retirements(network_id,address,previous_node_id,removed_revision,release_after) VALUES(?1,?2,?3,?4,NULL)",params![network.as_uuid().as_bytes().as_slice(),address.to_string(),node.as_uuid().as_bytes().as_slice(),revision.get() as i64])?;
        }
    }
    for node in previous
        .nodes
        .keys()
        .filter(|id| !target.nodes.contains_key(id))
    {
        tx.execute("UPDATE nodes SET credentials_revoked_at=COALESCE(credentials_revoked_at,?1) WHERE network_id=?2 AND id=?3",params![now,network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()])?;
        retire_node_sessions(tx, network, *node, now)?;
    }
    Ok(())
}

/// Every non-revoked Node's current netmap after an apply: the version's
/// fixed participants, including offline Nodes.
pub(crate) fn netmap_targets(
    tx: &Transaction<'_>,
    network: NetworkId,
    configuration: &NetworkConfiguration,
) -> Result<BTreeMap<NodeId, NetmapRef>, Error> {
    let mut targets = BTreeMap::new();
    for node in configuration.nodes.keys() {
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE network_id=?1 AND id=?2 AND credentials_revoked_at IS NULL)",
            params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        if !active {
            continue;
        }
        if let Some(current) = crate::store::netmap::current(tx, network, *node)? {
            targets.insert(*node, current.current);
        }
    }
    Ok(targets)
}
