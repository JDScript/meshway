//! Per-node netmap compilation from one configuration revision.
//!
//! Grants are compiled per (source, destination) pair: the Access union, every
//! routing segment's first match, topology carriers and exit eligibility.
//! Dynamic facts (instances, endpoints, exit readiness, demands, observation
//! tasks) are not inputs; no per-request compilation exists here.
use super::*;
use meshway_configuration::netmap::*;
use meshway_model::{Digest, NetmapRef, Version, netmap::MAX_NETMAP_BYTES};
use std::collections::BTreeSet;
mod grants;
mod local;

/// Stable inputs only. The caller loads them in the committing transaction.
pub struct NetmapInput<'a> {
    pub network: NetworkId,
    pub revision: Revision,
    pub configuration: &'a NetworkConfiguration,
    pub subjects: &'a SubjectContext,
    pub inventory: &'a BTreeMap<NodeId, TrustedNode>,
    pub rule_sets: &'a BTreeMap<String, RuleSetContent>,
    pub selected_profiles: &'a BTreeMap<NodeId, String>,
    /// Persisted selection versions; every profile switch increments one.
    pub selection_versions: &'a BTreeMap<NodeId, u64>,
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

    fn peer(&self, node: NodeId) -> Result<PeerConfiguration, CompileError> {
        let (Some(settings), Some(trusted)) = (
            self.configuration.nodes.get(&node),
            self.inventory.get(&node),
        ) else {
            return Err(CompileError::InvalidInput);
        };
        Ok(PeerConfiguration {
            identity: settings.identity.clone(),
            addresses: settings.addresses.clone(),
            control_key: trusted.control_key.clone(),
            dataplane_key: trusted.dataplane_key.clone(),
        })
    }

    fn observable(&self, nodes: &[NodeId]) -> bool {
        nodes.iter().all(|node| {
            self.inventory.get(node).is_some_and(|trusted| {
                trusted.capabilities.get(PATH_OBSERVATION_FEATURE)
                    == Some(&meshway_model::state::Capability::Feature { version: 1 })
            })
        })
    }

    pub(super) fn grant_path(&self, path: LogicalPath) -> GrantPath {
        GrantPath {
            observable: self.observable(&path.nodes),
            nodes: path.nodes,
            dial_candidates: path.dial_candidates,
        }
    }
}

/// Acquire capability that a path's every Node must declare to be observable.
pub const PATH_OBSERVATION_FEATURE: &str = "feature:path-observation";

/// Accumulates one Node's view while grants are distributed.
#[derive(Default)]
struct View {
    grants: BTreeMap<GrantKey, Grant>,
    /// Paths not carried by a grant: DNS service paths through this Node.
    service_paths: Vec<GrantPath>,
}

pub fn compile_netmap(input: NetmapInput<'_>) -> Result<CompiledNetmaps, NetmapError> {
    let cfg = input.configuration;
    if cfg.nodes.len() > 10_000 || input.inventory.len() > 10_000 {
        return Err(CompileError::Budget.into());
    }
    if input.rule_sets.values().any(|r| {
        r.network_id != input.network
            || r.ip_cidr.iter().any(|s| s.parse::<ipnet::IpNet>().is_err())
    }) {
        return Err(CompileError::InvalidInput.into());
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
    let dependency_digest = digest(&(
        meshway_configuration::netmap::NETMAP_COMPILER_VERSION,
        input.network,
        input.revision,
        cfg,
        input.subjects,
        input.inventory,
        input.rule_sets,
        input.selected_profiles,
        input.selection_versions,
    ))?;
    let graph = paths::Graph::new(input.topology())?;
    let mut locals = BTreeMap::new();
    for id in &members {
        locals.insert(*id, local::Local::compile(&input, *id)?);
    }
    let mut views: BTreeMap<NodeId, View> =
        members.iter().map(|id| (*id, View::default())).collect();
    for grant in grants::mesh(&input, &graph, &members)?
        .into_iter()
        .chain(grants::exit(&input, &graph, &members, &locals)?)
    {
        distribute(&mut views, grant)?;
    }
    let services = local::dns(&input, &graph, &mut locals)?;
    for service in services {
        for id in &service.path.nodes {
            views
                .get_mut(id)
                .ok_or(CompileError::InvalidInput)?
                .service_paths
                .push(service.path.clone());
        }
    }
    let mut nodes = BTreeMap::new();
    for id in members {
        let view = views.remove(&id).ok_or(CompileError::InvalidInput)?;
        let local = locals.remove(&id).ok_or(CompileError::InvalidInput)?;
        let compiled = assemble(&input, &graph, id, view, local, dependency_digest)?;
        nodes.insert(id, compiled);
    }
    Ok(CompiledNetmaps {
        network_id: input.network,
        revision: input.revision,
        compiler_version: meshway_configuration::netmap::NETMAP_COMPILER_VERSION,
        dependency_digest,
        nodes,
    })
}

/// Endpoints receive the full grant; relays receive only paths through them
/// and no service set, since they never inspect business traffic.
fn distribute(views: &mut BTreeMap<NodeId, View>, grant: Grant) -> Result<(), CompileError> {
    let key = grant.key();
    let terminal = match grant.target {
        GrantTarget::Mesh { node } | GrantTarget::Exit { node, .. } => node,
    };
    let participants: BTreeSet<NodeId> = grant
        .paths
        .iter()
        .flat_map(|path| path.nodes.iter().copied())
        .collect();
    for node in participants {
        let view = views.get_mut(&node).ok_or(CompileError::InvalidInput)?;
        let copy = if node == grant.source || node == terminal {
            grant.clone()
        } else {
            Grant {
                services: None,
                paths: grant
                    .paths
                    .iter()
                    .filter(|path| path.nodes.contains(&node))
                    .cloned()
                    .collect(),
                ..grant.clone()
            }
        };
        view.grants.insert(key, copy);
    }
    Ok(())
}

fn assemble(
    input: &NetmapInput<'_>,
    graph: &paths::Graph,
    id: NodeId,
    view: View,
    local: local::Local,
    dependency_digest: Digest,
) -> Result<CompiledNetmap, NetmapError> {
    let settings = &input.configuration.nodes[&id];
    let mut peers = BTreeMap::new();
    let mut edges: BTreeMap<NodeId, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let candidates = graph.candidates.borrow();
    let local_candidates = candidates.get(&id);
    for path in view
        .grants
        .values()
        .flat_map(|grant| &grant.paths)
        .chain(&view.service_paths)
    {
        for node in &path.nodes {
            if *node != id && !peers.contains_key(node) {
                peers.insert(*node, input.peer(*node)?);
            }
        }
        for pair in path.nodes.windows(2) {
            let peer = match (pair[0] == id, pair[1] == id) {
                (true, _) => pair[1],
                (_, true) => pair[0],
                _ => continue,
            };
            let (dial, accept) = edges.entry(peer).or_default();
            for key in path.dial_candidates.get(&id).into_iter().flatten() {
                if local_candidates
                    .and_then(|keys| keys.get(key))
                    .is_some_and(|candidate| candidate.peer == peer)
                {
                    dial.insert(key.clone());
                }
            }
            let prefix = format!("{id}/");
            for key in path.dial_candidates.get(&peer).into_iter().flatten() {
                if let Some(listener) = key.strip_prefix(&prefix) {
                    accept.insert(listener.to_owned());
                }
            }
        }
    }
    let mut dialers = BTreeMap::new();
    let edges = edges
        .into_iter()
        .map(|(peer, (dial, accept))| {
            let mut dial: Vec<String> = dial.into_iter().collect();
            for key in &dial {
                let candidate = local_candidates
                    .and_then(|keys| keys.get(key))
                    .ok_or(CompileError::InvalidInput)?;
                dialers.insert(key.clone(), candidate.clone());
            }
            dial.sort_by_key(|key| {
                let candidate = &dialers[key];
                (candidate.priority, candidate.listener_key.clone())
            });
            Ok((
                peer,
                Edges {
                    dial,
                    accept: accept.into_iter().collect(),
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CompileError>>()?;
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
        peers,
        dialers,
        edges,
        grants: view.grants,
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
    Ok(CompiledNetmap { digest, netmap })
}
