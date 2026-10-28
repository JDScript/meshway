//! Per-node netmap construction from one configuration revision.
//!
//! The Server projects the configuration for each Node and adds that Node's
//! own resources; it materializes no pair grant or path. Nodes run the same
//! compiler on the projection (`decide`). Dynamic facts (instances,
//! endpoints, exit readiness, demands, observation tasks) are not inputs.
use super::*;
use meshway_configuration::netmap::*;
use meshway_model::{Digest, NetmapRef, Version, netmap::MAX_NETMAP_BYTES};
use std::collections::BTreeSet;
mod associations;
mod decide;
mod incremental;
mod local;
mod observe;
mod project;
pub use associations::{Associations, associable};
pub use decide::{Decision, LocalCompiler, LocalDecision, Target, compile_local, transit};
pub use incremental::{IncrementalNetmaps, compile_incremental};
pub(crate) use local::resolvers as local_resolvers;
pub use observe::{ObservationDenied, ObservationPaths, observation_paths};

/// This Node's routing profile selection as its netmap decides it: the same
/// value the Server reports for the configuration the netmap projects.
pub fn routing_selection(
    netmap: &NodeNetmap,
) -> Result<meshway_configuration::selection::RoutingSelectionState, CompileError> {
    let projection = &netmap.projection;
    super::routing_selection(
        &decide::rebuild(&projection.configuration),
        &projection.subjects,
        projection
            .selected_profiles
            .get(&netmap.node_id)
            .map(String::as_str),
        projection.selection_versions.get(&netmap.node_id).copied(),
        netmap.node_id,
    )
}

/// Stable inputs only. The caller loads them in the committing transaction.
pub struct NetmapInput<'a> {
    pub network: NetworkId,
    pub revision: Revision,
    pub configuration: &'a NetworkConfiguration,
    pub subjects: &'a SubjectContext,
    pub inventory: &'a BTreeMap<NodeId, TrustedNode>,
    /// Loaded sets of every key a rule references (network keys and
    /// `<provider>:<name>`), inline sets included.
    pub rule_sets: &'a super::rules::RuleSets,
    pub selected_profiles: &'a BTreeMap<NodeId, String>,
    /// Persisted selection versions; every profile switch increments one.
    pub selection_versions: &'a BTreeMap<NodeId, meshway_model::Version>,
    /// Current stored netmap of each Node, used to retain serials of unchanged content.
    pub previous: &'a BTreeMap<NodeId, PreviousNetmap>,
}

/// The envelope of a Node's current stored netmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviousNetmap {
    pub reference: NetmapRef,
    pub dependency_digest: Digest,
}

impl From<&CompiledNetmap> for PreviousNetmap {
    fn from(compiled: &CompiledNetmap) -> Self {
        Self {
            reference: compiled.reference(),
            dependency_digest: compiled.netmap.dependency_digest,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetmapError {
    #[error(transparent)]
    Compile(#[from] CompileError),
    /// No partial netmap is ever produced; the whole compilation is rejected.
    #[error("netmap for node {node} is {bytes} bytes, above the safety bound")]
    Limit { node: NodeId, bytes: usize },
    #[error("netmap serial overflow")]
    Serial,
}

impl From<meshway_configuration::SerializationError> for NetmapError {
    fn from(_: meshway_configuration::SerializationError) -> Self {
        Self::Compile(CompileError::Serialization)
    }
}

/// Complete netmaps of every trusted configured Node for one revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledNetmaps {
    pub network_id: NetworkId,
    pub revision: Revision,
    pub compiler_version: &'static str,
    pub dependency_digest: Digest,
    pub nodes: BTreeMap<NodeId, CompiledNetmap>,
}

impl NetmapInput<'_> {
    fn topology(&self) -> Topology<'_> {
        Topology {
            configuration: self.configuration,
            subjects: self.subjects,
            inventory: self.inventory,
        }
    }

    fn profile(&self, node: NodeId) -> Result<String, CompileError> {
        effective_profile(
            self.configuration,
            self.subjects,
            self.selected_profiles,
            node,
        )
    }
}

/// Acquire capability that a path's every Node must declare to be observable.
pub const PATH_OBSERVATION_FEATURE: &str = "feature:path-observation";

/// Whether every Node of `nodes` declares path observation in `inventory`.
pub fn observable(inventory: &BTreeMap<NodeId, TrustedNode>, nodes: &[NodeId]) -> bool {
    nodes.iter().all(|node| {
        inventory.get(node).is_some_and(|trusted| {
            trusted.capabilities.get(PATH_OBSERVATION_FEATURE)
                == Some(&meshway_model::state::Capability::Feature { version: 1 })
        })
    })
}

fn validate(input: &NetmapInput<'_>) -> Result<Vec<NodeId>, NetmapError> {
    let cfg = input.configuration;
    if cfg.nodes.len() > 10_000 || input.inventory.len() > 10_000 {
        return Err(CompileError::Budget.into());
    }
    let members: Vec<NodeId> = input
        .inventory
        .keys()
        .copied()
        .filter(|id| cfg.nodes.contains_key(id))
        .collect();
    for id in &members {
        let trusted = &input.inventory[id];
        if !input.subjects.node_is_active(cfg, *id)
            || trusted.control_key.public_key.algorithm != meshway_model::KeyAlgorithm::Ed25519
            || trusted.dataplane_key.public_key.algorithm != meshway_model::KeyAlgorithm::X25519
        {
            return Err(CompileError::InvalidInput.into());
        }
    }
    Ok(members)
}

/// Projection of one Node, for callers that need it without the rest of its netmap.
pub fn project(
    input: &NetmapInput<'_>,
    node: NodeId,
) -> Result<ProjectedConfiguration, NetmapError> {
    let members = validate(input)?;
    let graph = paths::Graph::new(input.topology())?;
    let index = project::Index::new(input, &graph, &members)?;
    Ok(project::project(input, &index, node))
}

pub fn compile_netmap(input: NetmapInput<'_>) -> Result<CompiledNetmaps, NetmapError> {
    // Nothing is compared against these fingerprints.
    Ok(compile_incremental(input, &BTreeMap::new(), "")?.compiled)
}

fn compile_selected(
    input: &NetmapInput<'_>,
    graph: &paths::Graph<'_>,
    index: &project::Index,
    members: &[NodeId],
    selected: &BTreeSet<NodeId>,
) -> Result<CompiledNetmaps, NetmapError> {
    let cfg = input.configuration;
    let dependency_digest = digest(&(
        meshway_configuration::netmap::NETMAP_COMPILER_VERSION,
        input.network,
        input.revision,
        cfg,
        input.subjects,
        input.inventory,
        references(input.rule_sets),
        input.selected_profiles,
        input.selection_versions,
    ))?;
    let mut locals = BTreeMap::new();
    for id in selected {
        locals.insert(
            *id,
            local::Local::compile(input, matching::Exits::Full(input.configuration), *id)?,
        );
    }
    local::dns(input, graph, members, &mut locals)?;
    let mut nodes = BTreeMap::new();
    for &id in selected {
        let projection = project::project(input, index, id);
        let mut dialers = BTreeMap::new();
        for peer in projection.configuration.nodes.keys() {
            if *peer == id || !graph.topology_allows(id, *peer) {
                continue;
            }
            for candidate in graph.carriers(id, *peer)? {
                dialers.insert(format!("{peer}/{}", candidate.listener_key), candidate);
            }
        }
        let local = locals.remove(&id).ok_or(CompileError::InvalidInput)?;
        nodes.insert(
            id,
            assemble(input, id, projection, dialers, local, dependency_digest)?,
        );
    }
    Ok(CompiledNetmaps {
        network_id: input.network,
        revision: input.revision,
        compiler_version: meshway_configuration::netmap::NETMAP_COMPILER_VERSION,
        dependency_digest,
        nodes,
    })
}

fn assemble(
    input: &NetmapInput<'_>,
    id: NodeId,
    projection: ProjectedConfiguration,
    dialers: BTreeMap<String, DialCandidate>,
    local: local::Local,
    dependency_digest: Digest,
) -> Result<CompiledNetmap, NetmapError> {
    let settings = &input.configuration.nodes[&id];
    let enabled = settings.enabled;
    let mut netmap = NodeNetmap {
        network_id: input.network,
        node_id: id,
        revision: input.revision,
        serial: Version::MIN,
        compiler_version: meshway_configuration::netmap::NETMAP_COMPILER_VERSION.into(),
        dependency_digest,
        self_: NetmapSelf {
            enabled,
            hostname: settings.hostname.clone(),
            identity: settings.identity.clone(),
            addresses: settings.addresses.clone(),
            self_keys: input.inventory[&id].clone(),
            mesh_address_pools: input.configuration.address_pools.clone(),
            listeners: if enabled {
                settings.listeners.clone()
            } else {
                BTreeMap::new()
            },
            exit_enabled: enabled && settings.exit_enabled,
        },
        projection,
        dialers,
        egress: enabled.then_some(local.egress),
        exit_role: local.exit_role,
        dns: local.dns,
    };
    let digest = netmap.content_digest()?;
    if let Some(previous) = input.previous.get(&id) {
        if previous.reference.digest == digest {
            netmap.revision = previous.reference.revision;
            netmap.serial = previous.reference.serial;
            netmap.dependency_digest = previous.dependency_digest;
        } else {
            netmap.serial = previous
                .reference
                .serial
                .checked_next()
                .map_err(|_| NetmapError::Serial)?;
        }
    }
    let bytes = netmap.encoded_len()?;
    if bytes > MAX_NETMAP_BYTES {
        return Err(NetmapError::Limit { node: id, bytes });
    }
    Ok(CompiledNetmap {
        digest,
        netmap: std::sync::Arc::new(netmap),
    })
}

/// The content identity of loaded sets, for digests over compiler inputs.
fn references(sets: &super::rules::RuleSets) -> BTreeMap<&str, meshway_rules::Digest> {
    sets.iter()
        .map(|(key, set)| (key.as_str(), set.digest()))
        .collect()
}
