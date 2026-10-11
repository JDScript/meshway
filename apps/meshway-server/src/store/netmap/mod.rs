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
use meshway_policy::compile::netmap::{
    NetmapError, NetmapInput, PreviousNetmap, compile_incremental,
};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

mod inputs;

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
    let rule_sets =
        crate::resources::bindings::compilation_contents(tx, network, rule_set_bindings)?;
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
        if !head.withdrawn && !compiled.inputs.contains_key(node) {
            tx.execute(
                "UPDATE node_netmaps SET withdrawn_at=?3 WHERE network_id=?1 AND node_id=?2 AND withdrawn_at IS NULL",
                params![bytes(network.as_uuid()), bytes(node.as_uuid()), now],
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

/// One stored netmap signing key: its public binding, when it was created
/// (the `notBefore` of the handover to it) and its sealed private seed.
pub(crate) struct SigningKeyRow {
    pub(crate) public: meshway_protocol::netmap::ServerSigningKey,
    pub(crate) created_at: i64,
    sealed: Vec<u8>,
}

impl SigningKeyRow {
    /// Opens the sealed seed; it must derive the stored public key.
    pub(crate) fn seed(
        &self,
        sealer: &Sealer,
        public_of: impl Fn(&[u8; 32]) -> [u8; 32],
    ) -> Result<zeroize::Zeroizing<[u8; 32]>, Error> {
        let plaintext = sealer.open_signing_key(
            self.public.key_id,
            self.public.key_generation,
            &self.sealed,
        )?;
        let mut seed = zeroize::Zeroizing::new([0; 32]);
        if plaintext.as_bytes().len() != 32 {
            return Err(Error::decode("server_signing_keys.private_key"));
        }
        seed.copy_from_slice(plaintext.as_bytes());
        if &public_of(&seed) != self.public.public_key.bytes.as_bytes() {
            return Err(Error::decode("server_signing_keys.private_key"));
        }
        Ok(seed)
    }
}

/// The keys that sign sync answers at one instant: the current key and,
/// while its rotation window is open, the key it replaces.
pub(crate) struct SigningKeys {
    pub(crate) current: SigningKeyRow,
    /// The previous generation whose `retired_at` (the end of its rotation
    /// window) is still in the future.
    pub(crate) rotating: Option<(SigningKeyRow, i64)>,
}

type KeyColumns = (Vec<u8>, i64, Vec<u8>, Vec<u8>, i64, Option<i64>);

fn key_row(
    (key_id, generation, public, sealed, created_at, _): KeyColumns,
) -> Result<SigningKeyRow, Error> {
    Ok(SigningKeyRow {
        public: public_signing_key(
            id(key_id)?,
            u64::try_from(generation)
                .ok()
                .and_then(|value| Version::new(value).ok())
                .ok_or(Error::decode("server_signing_keys.generation"))?,
            public
                .try_into()
                .map_err(|_| Error::decode("server_signing_keys.public_key"))?,
        ),
        created_at,
        sealed,
    })
}

const KEY_COLUMNS: &str =
    "SELECT id,generation,public_key,private_key,created_at,retired_at FROM server_signing_keys";

fn key_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeyColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

/// The signing keys in effect at `now`; an internal fault before the first key exists.
pub(crate) fn signing_keys(tx: &Transaction<'_>, now: i64) -> Result<SigningKeys, Error> {
    let current = current_key(tx)?;
    let rotating = tx
        .query_row(
            &format!("{KEY_COLUMNS} WHERE purpose='netmap' AND generation=?1 AND retired_at>?2"),
            params![current.public.key_generation.get() as i64 - 1, now],
            key_columns,
        )
        .optional()?
        .map(|columns| {
            let retires_at = columns
                .5
                .ok_or(Error::decode("server_signing_keys.retired_at"))?;
            key_row(columns).map(|row| (row, retires_at))
        })
        .transpose()?;
    Ok(SigningKeys { current, rotating })
}

fn current_key(tx: &Transaction<'_>) -> Result<SigningKeyRow, Error> {
    key_row(
        tx.query_row(
            &format!("{KEY_COLUMNS} WHERE purpose='netmap' AND retired_at IS NULL"),
            [],
            key_columns,
        )
        .optional()?
        .ok_or(Error::Internal("netmap signing key missing"))?,
    )
}

/// Creates generation 1 when no netmap signing key exists yet and checks that
/// the keys in effect open and match their public halves. The private seed
/// never leaves the sealed column in plaintext.
pub(crate) fn ensure_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    now: i64,
    generate: impl FnOnce() -> Result<zeroize::Zeroizing<[u8; 32]>, Error>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<(), Error> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM server_signing_keys WHERE purpose='netmap')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        insert_signing_key(tx, sealer, Version::MIN, now, generate()?, &public_of)?;
    }
    let keys = signing_keys(tx, now)?;
    keys.current.seed(sealer, &public_of)?;
    if let Some((rotating, _)) = &keys.rotating {
        rotating.seed(sealer, &public_of)?;
    }
    Ok(())
}

fn insert_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    generation: Version,
    now: i64,
    seed: zeroize::Zeroizing<[u8; 32]>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<meshway_protocol::netmap::ServerSigningKey, Error> {
    let key_id = crate::secrets::random_key_id()?;
    let public = public_of(&seed);
    let sealed = sealer.seal_signing_key(key_id, generation, seed.as_slice())?;
    tx.execute(
        "INSERT INTO server_signing_keys(id,purpose,generation,public_key,private_key,created_at)
         VALUES(?1,'netmap',?2,?3,?4,?5)",
        params![
            bytes(key_id.as_uuid()),
            generation.get() as i64,
            public.as_slice(),
            sealed.as_bytes(),
            now
        ],
    )?;
    Ok(public_signing_key(key_id, generation, public))
}

/// One committed rotation of the netmap signing key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotatedSigningKey {
    /// The key that endorses its successor until the window ends.
    pub previous: meshway_protocol::netmap::ServerSigningKey,
    /// The new current key; Nodes enrolled from now on pin it directly.
    pub next: meshway_protocol::netmap::ServerSigningKey,
    /// When the handover window ends, in Unix milliseconds.
    pub window_ends_at_ms: u64,
}

/// Starts a rotation: a new key of the next generation becomes current at
/// `now` (its `notBefore`) and the replaced key's `retired_at` is set to the
/// end of the window. Only one window may be open at a time.
pub(crate) fn rotate_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    now: i64,
    window_ms: i64,
    generate: impl FnOnce() -> Result<zeroize::Zeroizing<[u8; 32]>, Error>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<RotatedSigningKey, Error> {
    let keys = signing_keys(tx, now)?;
    if keys.rotating.is_some() {
        return Err(Error::StateConflict);
    }
    let previous = keys.current.public;
    let generation = previous
        .key_generation
        .checked_next()
        .map_err(|_| Error::Internal("netmap signing key generation overflow"))?;
    let ends = now.checked_add(window_ms).ok_or(Error::InvalidRequest)?;
    tx.execute(
        "UPDATE server_signing_keys SET retired_at=?2 WHERE id=?1 AND retired_at IS NULL",
        params![bytes(previous.key_id.as_uuid()), ends],
    )?;
    let next = insert_signing_key(tx, sealer, generation, now, generate()?, public_of)?;
    Ok(RotatedSigningKey {
        previous,
        next,
        window_ends_at_ms: u64::try_from(ends).map_err(|_| Error::InvalidRequest)?,
    })
}

/// How a Node's pinned key generation relates to the keys in effect at `now`.
pub(crate) fn pinned_status(
    tx: &Transaction<'_>,
    now: i64,
    generation: Version,
) -> Result<meshway_protocol::PinnedServerKeyStatus, Error> {
    use meshway_protocol::PinnedServerKeyStatus as Status;
    let keys = signing_keys(tx, now)?;
    Ok(if generation == keys.current.public.key_generation {
        Status::Current
    } else if keys
        .rotating
        .is_some_and(|(row, _)| row.public.key_generation == generation)
    {
        Status::Rotating
    } else {
        Status::KeyMismatch
    })
}

/// Public half of the current netmap signing key, for enrollment responses.
pub(crate) fn server_signing_key(
    tx: &Transaction<'_>,
) -> Result<meshway_protocol::netmap::ServerSigningKey, Error> {
    Ok(current_key(tx)?.public)
}

fn public_signing_key(
    key_id: meshway_model::KeyId,
    key_generation: Version,
    public: [u8; 32],
) -> meshway_protocol::netmap::ServerSigningKey {
    meshway_protocol::netmap::ServerSigningKey {
        key_id,
        key_generation,
        public_key: meshway_model::NodePublicKey {
            algorithm: meshway_model::KeyAlgorithm::Ed25519,
            bytes: Digest::from_bytes(public),
        },
    }
}
