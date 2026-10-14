//! Per-Node netmaps: recompilation inside the transaction that changes a
//! stable input, and the stored current and previous content of each Node.
//!
//! Only stable inputs reach `compile_netmap`: configuration, subjects, trusted
//! keys and capabilities, rule-set content and profile selections. State
//! reports, instance renewal and expiry, and observation subscriptions never
//! call into this module.
use super::execution::{CompilationContext, compilation_context, current_configuration, id};
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
use meshway_policy::compile::netmap::{
    NetmapError, NetmapInput, PreviousNetmap, compile_incremental,
};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

mod inputs;
mod signing;
pub use signing::RotatedSigningKey;
pub(crate) use signing::{
    SigningKeyRow, ensure_signing_key, pinned_status, rotate_signing_key, server_signing_key,
    signing_keys,
};

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

/// What changed a stable input, deciding how a netmap above the bound ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// A configuration version or rule-set binding: the whole transaction is
    /// rejected with the `netmap_limit` configuration issue.
    Configuration,
    /// A Node-side or credential operation (key revocation and rotation,
    /// capabilities, profile selection) that must not be refused because of a
    /// netmap: each over-limit Node is excluded from the inventory, its netmap
    /// becomes unavailable and every other Node is compiled without it.
    Node,
}

/// How many over-limit Nodes one Node-triggered recompilation excludes before
/// it gives up and the operation itself is unavailable.
const MAX_EXCLUDED: usize = 16;

/// Recompiles affected Nodes from the current stable inputs and the current
/// rule-set binding. Fingerprints and output changes share this transaction.
pub(crate) fn recompile(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
    trigger: Trigger,
) -> Result<Recompiled, Error> {
    let (revision, _, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets =
        crate::resources::bindings::at_revision(tx, network, revision, &bindings.rule_sets)?;
    recompile_with_rule_sets(tx, sealer, network, now, &rule_sets, trigger)
}

/// Same as [`recompile`] with the rule-set binding the caller is committing.
pub(crate) fn recompile_with_rule_sets(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
    rule_set_bindings: &crate::resources::bindings::Bindings,
    trigger: Trigger,
) -> Result<Recompiled, Error> {
    let (revision, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let rule_sets = crate::resources::bindings::contents(tx, rule_set_bindings)?;
    let CompilationContext {
        subjects,
        inventory,
        selected_profiles,
    } = compilation_context(tx, network, &cfg, &bindings)?;
    let selection_versions = selection_versions(tx, network, &cfg, &bindings)?;
    let heads = heads(tx, network)?;
    let previous_inputs = inputs::load(tx, network)?;
    let previous: BTreeMap<NodeId, PreviousNetmap> = heads
        .iter()
        .map(|(node, head)| (*node, head.previous))
        .collect();
    let mut excluded = BTreeSet::new();
    let compiled = loop {
        let mut trusted = inventory.clone();
        trusted.retain(|node, _| !excluded.contains(node));
        match compile_incremental(
            NetmapInput {
                network,
                revision,
                configuration: &cfg,
                subjects: &subjects,
                inventory: &trusted,
                rule_sets: &rule_sets,
                selected_profiles: &selected_profiles,
                selection_versions: &selection_versions,
                previous: &previous,
            },
            &previous_inputs,
        ) {
            Ok(compiled) => break compiled,
            Err(NetmapError::Limit { node, bytes }) if trigger == Trigger::Node => {
                tracing::warn!(network = %network, node = %node, bytes, "Netmap exceeds the safety bound; the Node's netmap is unavailable");
                if let Ok(mut sizes) = LIMIT_BYTES.lock() {
                    sizes.insert((network, node), u32::try_from(bytes).unwrap_or(u32::MAX));
                }
                if excluded.len() >= MAX_EXCLUDED || !excluded.insert(node) {
                    return Err(Error::Unavailable(crate::error::Unavailability::Netmap));
                }
            }
            Err(error) => return Err(netmap_error(error, trigger)),
        }
    };
    tx.execute(
        "DELETE FROM node_netmap_unavailable WHERE network_id=?1",
        [bytes(network.as_uuid())],
    )?;
    for node in &excluded {
        tx.execute(
            "INSERT INTO node_netmap_unavailable(network_id,node_id,reason,since) VALUES(?1,?2,'limit',?3)",
            params![bytes(network.as_uuid()), bytes(node.as_uuid()), now],
        )?;
    }
    let mut changed = BTreeSet::new();
    for (node, netmap) in compiled.compiled.nodes {
        let reference = netmap.reference();
        match heads.get(&node) {
            Some(head) if head.previous.reference == reference => {
                if head.withdrawn {
                    tx.execute(
                        "UPDATE node_netmaps SET withdrawn_at=NULL WHERE network_id=?1 AND node_id=?2",
                        params![bytes(network.as_uuid()), bytes(node.as_uuid())],
                    )?;
                    super::resources::artifacts::restore(
                        tx,
                        bytes(network.as_uuid()),
                        bytes(node.as_uuid()),
                        reference.serial.get() as i64,
                    )?;
                    changed.insert(node);
                }
            }
            _ => {
                let previous = heads.get(&node).map(|head| head.previous.reference);
                record_auto_resolution(tx, sealer, network, node, previous, &netmap, now)?;
                insert(tx, sealer, network, node, &netmap, now)?;
                changed.insert(node);
            }
        }
    }
    for (node, head) in &heads {
        if !head.withdrawn && !compiled.inputs.contains_key(node) {
            tx.execute(
                "UPDATE node_netmaps SET withdrawn_at=?3 WHERE network_id=?1 AND node_id=?2 AND withdrawn_at IS NULL",
                params![bytes(network.as_uuid()), bytes(node.as_uuid()), now],
            )?;
            super::resources::artifacts::withdraw(
                tx,
                bytes(network.as_uuid()),
                bytes(node.as_uuid()),
                now,
            )?;
            changed.insert(*node);
        }
    }
    inputs::save(tx, network, &previous_inputs, &compiled.inputs)?;
    Ok(Recompiled {
        network: Some(network),
        changed,
    })
}

/// A `routing.autoResolve` activity (Server actor) when the new netmap
/// applies `auto` and resolves it differently from the Node's previous
/// netmap: another profile, basis or region. Both sides are read from the
/// netmaps with the same `routing_selection` the Node shows.
fn record_auto_resolution(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    previous: Option<NetmapRef>,
    netmap: &CompiledNetmap,
    now: i64,
) -> Result<(), Error> {
    let resolution = |netmap: &CompiledNetmap| {
        meshway_policy::compile::netmap::routing_selection(&netmap.netmap)
            .map(|selection| selection.auto)
    };
    let Some(current) = resolution(netmap)? else {
        return Ok(());
    };
    if let Some(previous) = previous
        && resolution(&load(tx, sealer, network, node, previous)?)?.as_ref() == Some(&current)
    {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO activity_events (id, scope, network_id, at, actor_kind, action, request_id, target_node_id, target_profile) VALUES (?1, 'network', ?2, ?3, 'system', 'routing.autoResolve', ?4, ?5, ?6)",
        params![
            crate::accounts::random_uuid()?.as_bytes().as_slice(),
            bytes(network.as_uuid()),
            now,
            crate::accounts::random_uuid()?.as_bytes().as_slice(),
            bytes(node.as_uuid()),
            current.profile,
        ],
    )?;
    Ok(())
}

/// Compiler inputs are the Server's own committed data: an invalid one is an
/// internal fault. A budget is deterministic: a configuration that exceeds it
/// is invalid, and a Node-side change that hits it leaves netmaps unavailable
/// rather than inviting endless retries.
fn netmap_error(error: NetmapError, trigger: Trigger) -> Error {
    match error {
        NetmapError::Compile(meshway_policy::compile::CompileError::InvalidInput) => {
            Error::Internal("netmap compiler input")
        }
        NetmapError::Compile(meshway_policy::compile::CompileError::Budget) => match trigger {
            Trigger::Configuration => {
                Error::InvalidConfiguration(ConfigurationError::Invalid(vec![
                    ConfigurationIssue::new(String::new(), IssueCode::NetmapBudget),
                ]))
            }
            Trigger::Node => Error::Unavailable(crate::error::Unavailability::Netmap),
        },
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
    Ok(super::execution::selections(tx, network, cfg, bindings)?
        .into_iter()
        .map(|(node, (_, version))| (node, version))
        .collect())
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
        "INSERT INTO node_netmap_history(network_id,node_id,serial,revision,digest) VALUES(?1,?2,?3,?4,?5)",
        params![bytes(network.as_uuid()), bytes(node.as_uuid()), serial.get() as i64,
            netmap.netmap.revision.get() as i64, netmap.digest.as_bytes().as_slice()],
    )?;
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
    // The artifact endpoint authorizes exactly the digests of this Node's
    // netmaps that can still be under a lease.
    super::resources::artifacts::reference(
        tx,
        bytes(network.as_uuid()),
        bytes(node.as_uuid()),
        serial.get() as i64,
        netmap
            .netmap
            .projection
            .rule_sets
            .values()
            .map(|reference| reference.digest),
        now,
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

/// Whether the Node's netmap was excluded for exceeding the safety bound.
pub(crate) fn unavailable(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM node_netmap_unavailable WHERE network_id=?1 AND node_id=?2)",
        params![bytes(network.as_uuid()), bytes(node.as_uuid())],
        |row| row.get(0),
    )?)
}

/// Measured sizes of netmaps excluded for the bound, as this process saw
/// them; display only (the durable fact is `node_netmap_unavailable`).
static LIMIT_BYTES: std::sync::LazyLock<std::sync::Mutex<BTreeMap<(NetworkId, NodeId), u32>>> =
    std::sync::LazyLock::new(Default::default);

/// Why `node` has no current netmap; None when it has one.
pub(crate) fn unavailability(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<Option<meshway_protocol::node::NetmapUnavailable>, Error> {
    use meshway_protocol::node::NetmapUnavailable;
    if current(tx, network, node)?.is_some() {
        return Ok(None);
    }
    if unavailable(tx, network, node)? {
        let bytes = LIMIT_BYTES
            .lock()
            .ok()
            .and_then(|sizes| sizes.get(&(network, node)).copied());
        return Ok(Some(NetmapUnavailable::Limit {
            bytes,
            max_bytes: u32::try_from(MAX_NETMAP_BYTES).unwrap_or(u32::MAX),
        }));
    }
    let issued: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM node_netmaps WHERE network_id=?1 AND node_id=?2)",
        params![bytes(network.as_uuid()), bytes(node.as_uuid())],
        |row| row.get(0),
    )?;
    Ok(Some(if issued {
        NetmapUnavailable::Withdrawn
    } else {
        NetmapUnavailable::NotIssued
    }))
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

/// Upper bound of verified netmaps kept decrypted; reaching it starts over.
const MAX_CACHED: usize = 16_384;

/// The production netmap reader: each Node's current stored netmap, read in
/// the caller's transaction and decrypted/verified once per content. A cached
/// value is returned only while it is still the stored current reference.
pub(crate) struct StoredNetmaps {
    sealer: std::sync::Arc<Sealer>,
    cache: std::sync::Mutex<BTreeMap<(NetworkId, NodeId), std::sync::Arc<CompiledNetmap>>>,
}

impl StoredNetmaps {
    pub(crate) fn new(sealer: std::sync::Arc<Sealer>) -> Self {
        Self {
            sealer,
            cache: std::sync::Mutex::default(),
        }
    }
}

impl crate::state::facts::NetmapReader for StoredNetmaps {
    fn current(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Option<std::sync::Arc<CompiledNetmap>>, Error> {
        let Some(Current {
            current: expected, ..
        }) = current(tx, network, node)?
        else {
            return Ok(None);
        };
        if let Ok(cache) = self.cache.lock()
            && let Some(netmap) = cache.get(&(network, node))
            && netmap.reference() == expected
        {
            return Ok(Some(std::sync::Arc::clone(netmap)));
        }
        let netmap = std::sync::Arc::new(load(tx, &self.sealer, network, node, expected)?);
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= MAX_CACHED {
                cache.clear();
            }
            cache.insert((network, node), std::sync::Arc::clone(&netmap));
        }
        Ok(Some(netmap))
    }
}

/// A [`StoredNetmaps`] bound once the database (and its sealer) is open. Until
/// then no Node has a netmap, so nothing is recorded or disclosed.
#[derive(Default)]
pub(crate) struct LateNetmaps(std::sync::OnceLock<StoredNetmaps>);

impl LateNetmaps {
    pub(crate) fn bind(&self, netmaps: StoredNetmaps) {
        let _ = self.0.set(netmaps);
    }
}

impl crate::state::facts::NetmapReader for LateNetmaps {
    fn current(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Option<std::sync::Arc<CompiledNetmap>>, Error> {
        match self.0.get() {
            Some(netmaps) => netmaps.current(tx, network, node),
            None => Ok(None),
        }
    }
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
