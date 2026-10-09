//! Per-Node netmaps: recompilation inside the transaction that changes a
//! stable input, and the stored current and previous content of each Node.
//!
//! Only stable inputs reach `compile_netmap`: configuration, subjects, trusted
//! keys and capabilities, rule-set content and profile selections. State
//! reports, instance renewal and expiry, and observation subscriptions never
//! call into this module.
use super::execution::{
    CompilationContext, compilation_context, current_configuration, id, identity_digest,
};
use crate::effect::Effect;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_configuration::{
    ConfigurationError, ConfigurationIssue, IssueCode, NetworkConfiguration,
};
use meshway_model::{
    Digest, NetmapRef, NetworkId, NodeId, Revision, Version, netmap::MAX_NETMAP_BYTES,
};
use meshway_policy::compile::netmap::{NetmapError, NetmapInput, PreviousNetmap, compile_netmap};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

/// Nodes whose current netmap changed in one transaction.
#[derive(Debug, Default)]
pub(crate) struct Recompiled {
    pub(crate) network: Option<NetworkId>,
    pub(crate) changed: BTreeSet<NodeId>,
}

impl Recompiled {
    /// The post-commit wake-up of the changed Nodes' pending `sync`; none when nothing changed.
    pub(crate) fn effects(&self) -> Vec<Effect> {
        match self.network {
            Some(network_id) if !self.changed.is_empty() => vec![Effect::Netmap {
                network_id,
                node_ids: self.changed.clone(),
            }],
            _ => Vec::new(),
        }
    }
}

/// Recompiles every Node of `network` from the current stable inputs and the
/// current rule-set binding.
pub(crate) fn recompile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
) -> Result<Recompiled, Error> {
    let (revision, _, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets =
        crate::resources::bindings::at_revision(tx, network, revision, &bindings.rule_sets)?;
    recompile_with_rule_sets(tx, sealer, network, now, &rule_sets)
}

/// Same as [`recompile`] with the rule-set binding the caller is committing.
pub(crate) fn recompile_with_rule_sets(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
    rule_set_bindings: &crate::resources::bindings::Bindings,
) -> Result<Recompiled, Error> {
    let (revision, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets =
        crate::resources::bindings::compilation_contents(tx, network, rule_set_bindings)?;
    let CompilationContext {
        subjects,
        inventory,
        selected_profiles,
    } = compilation_context(tx, network, &cfg, &bindings)?;
    let selection_versions = selection_versions(tx, network, &cfg, &bindings)?;
    let heads = heads(tx, network)?;
    let previous: BTreeMap<NodeId, PreviousNetmap> = heads
        .iter()
        .map(|(node, head)| (*node, head.previous))
        .collect();
    let compiled = compile_netmap(NetmapInput {
        network,
        revision,
        configuration: &cfg,
        subjects: &subjects,
        inventory: &inventory,
        rule_sets: &rule_sets,
        selected_profiles: &selected_profiles,
        selection_versions: &selection_versions,
        previous: &previous,
    })
    .map_err(netmap_error)?;
    let mut changed = BTreeSet::new();
    let compiled_nodes: BTreeSet<NodeId> = compiled.nodes.keys().copied().collect();
    for (node, netmap) in compiled.nodes {
        let reference = netmap.reference();
        match heads.get(&node) {
            Some(head) if head.previous.reference == reference => {
                if head.withdrawn {
                    tx.execute(
                        "UPDATE node_netmaps SET withdrawn_at=NULL WHERE network_id=?1 AND node_id=?2",
                        params![bytes(network.as_uuid()), bytes(node.as_uuid())],
                    )?;
                    changed.insert(node);
                }
            }
            _ => {
                insert(tx, sealer, network, node, &netmap, now)?;
                changed.insert(node);
            }
        }
    }
    for (node, head) in &heads {
        if !head.withdrawn && !compiled_nodes.contains(node) {
            tx.execute(
                "UPDATE node_netmaps SET withdrawn_at=?3 WHERE network_id=?1 AND node_id=?2 AND withdrawn_at IS NULL",
                params![bytes(network.as_uuid()), bytes(node.as_uuid()), now],
            )?;
            changed.insert(*node);
        }
    }
    Ok(Recompiled {
        network: Some(network),
        changed,
    })
}

fn netmap_error(error: NetmapError) -> Error {
    match error {
        NetmapError::Compile(error) => error.into(),
        NetmapError::Limit { node, bytes } => {
            Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
                ConfigurationIssue::new(format!("/nodes/{node}"), IssueCode::NetmapLimit)
                    .with("node", node)
                    .with("bytes", bytes)
                    .with("maxBytes", MAX_NETMAP_BYTES),
            ]))
        }
        NetmapError::Serial => Error::Internal("netmap serial overflow"),
    }
}

/// Persisted selection versions of Nodes whose selection matches their current identity.
fn selection_versions(
    tx: &Transaction<'_>,
    network: NetworkId,
    cfg: &NetworkConfiguration,
    bindings: &crate::configuration::ConfigurationBindings,
) -> Result<BTreeMap<NodeId, Version>, Error> {
    let mut statement = tx.prepare(
        "SELECT node_id,identity_digest,version FROM routing_selections WHERE network_id=?1",
    )?;
    let mut rows = statement.query([bytes(network.as_uuid())])?;
    let mut versions = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let node: NodeId = id(row.get(0)?)?;
        let stored: Vec<u8> = row.get(1)?;
        if cfg.nodes.contains_key(&node)
            && stored == identity_digest(cfg, bindings, node)?.as_bytes()
        {
            let version: i64 = row.get(2)?;
            versions.insert(
                node,
                u64::try_from(version)
                    .ok()
                    .and_then(|version| Version::new(version).ok())
                    .ok_or(Error::decode("routing_selections.version"))?,
            );
        }
    }
    Ok(versions)
}

struct Head {
    previous: PreviousNetmap,
    withdrawn: bool,
}

/// The highest stored serial of every Node, withdrawn or not.
fn heads(tx: &Transaction<'_>, network: NetworkId) -> Result<BTreeMap<NodeId, Head>, Error> {
    let mut statement = tx.prepare(
        "SELECT node_id,revision,serial,digest,dependency_digest,withdrawn_at IS NOT NULL
         FROM node_netmaps x WHERE network_id=?1 AND serial=(
           SELECT max(serial) FROM node_netmaps y WHERE y.network_id=x.network_id AND y.node_id=x.node_id)",
    )?;
    let mut rows = statement.query([bytes(network.as_uuid())])?;
    let mut heads = BTreeMap::new();
    while let Some(row) = rows.next()? {
        heads.insert(
            id(row.get(0)?)?,
            Head {
                previous: PreviousNetmap {
                    reference: reference(row.get(1)?, row.get(2)?, row.get(3)?)?,
                    dependency_digest: digest(row.get(4)?)?,
                },
                withdrawn: row.get(5)?,
            },
        );
    }
    Ok(heads)
}

fn insert(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    netmap: &CompiledNetmap,
    now: i64,
) -> Result<(), Error> {
    let serial = netmap.netmap.serial;
    let payload =
        zeroize::Zeroizing::new(serde_json::to_vec(netmap).map_err(|_| Error::encode("netmap"))?);
    let sealed = sealer.seal_netmap(network, node, serial, &payload)?;
    tx.execute(
        "INSERT INTO node_netmaps(network_id,node_id,serial,revision,digest,dependency_digest,payload,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            bytes(network.as_uuid()),
            bytes(node.as_uuid()),
            serial.get() as i64,
            netmap.netmap.revision.get() as i64,
            netmap.digest.as_bytes().as_slice(),
            netmap.netmap.dependency_digest.as_bytes().as_slice(),
            sealed.as_bytes(),
            now,
        ],
    )?;
    // Keep the new head and the one delta base before it.
    tx.execute(
        "DELETE FROM node_netmaps WHERE network_id=?1 AND node_id=?2 AND serial<?3",
        params![
            bytes(network.as_uuid()),
            bytes(node.as_uuid()),
            serial.get() as i64 - 1
        ],
    )?;
    Ok(())
}

/// The Node's current netmap and the delta base kept before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Current {
    pub(crate) current: NetmapRef,
    pub(crate) previous: Option<NetmapRef>,
}

/// Null when the Node has no current netmap (never compiled, or withdrawn).
pub(crate) fn current(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<Option<Current>, Error> {
    let mut statement = tx.prepare(
        "SELECT revision,serial,digest,withdrawn_at IS NOT NULL FROM node_netmaps
         WHERE network_id=?1 AND node_id=?2 ORDER BY serial DESC LIMIT 2",
    )?;
    let rows = statement
        .query_map(
            params![bytes(network.as_uuid()), bytes(node.as_uuid())],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = rows.into_iter();
    let Some((revision, serial, head_digest, withdrawn)) = rows.next() else {
        return Ok(None);
    };
    if withdrawn {
        return Ok(None);
    }
    let previous = rows
        .next()
        .map(|(revision, serial, digest, _)| reference(revision, serial, digest))
        .transpose()?;
    Ok(Some(Current {
        current: reference(revision, serial, head_digest)?,
        previous,
    }))
}

/// Decrypts and verifies one stored netmap; it must be exactly `expected`.
pub(crate) fn load(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    expected: NetmapRef,
) -> Result<CompiledNetmap, Error> {
    let payload: Vec<u8> = tx
        .query_row(
            "SELECT payload FROM node_netmaps WHERE network_id=?1 AND node_id=?2 AND serial=?3",
            params![
                bytes(network.as_uuid()),
                bytes(node.as_uuid()),
                expected.serial.get() as i64
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::decode("node_netmaps.payload"))?;
    let plaintext = sealer.open_netmap(network, node, expected.serial, &payload)?;
    let netmap: CompiledNetmap = serde_json::from_slice(plaintext.as_bytes())
        .map_err(|_| Error::decode("node_netmaps.payload"))?;
    if netmap.verify().is_err()
        || netmap.reference() != expected
        || netmap.netmap.network_id != network
        || netmap.netmap.node_id != node
    {
        return Err(Error::decode("node_netmaps.payload"));
    }
    Ok(netmap)
}

fn bytes(id: &uuid::Uuid) -> &[u8] {
    id.as_bytes()
}

fn digest(value: Vec<u8>) -> Result<Digest, Error> {
    Ok(Digest::from_bytes(
        value
            .try_into()
            .map_err(|_| Error::decode("node_netmaps.digest"))?,
    ))
}

fn reference(revision: i64, serial: i64, content: Vec<u8>) -> Result<NetmapRef, Error> {
    let number = |value: i64, field| u64::try_from(value).map_err(|_| Error::decode(field));
    Ok(NetmapRef {
        revision: Revision::new(number(revision, "node_netmaps.revision")?)
            .map_err(|_| Error::decode("node_netmaps.revision"))?,
        serial: Version::new(number(serial, "node_netmaps.serial")?)
            .map_err(|_| Error::decode("node_netmaps.serial"))?,
        digest: digest(content)?,
    })
}
