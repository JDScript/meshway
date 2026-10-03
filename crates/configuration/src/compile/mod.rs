//! Pure bounded demand compilation from one trusted input snapshot.
//! The caller authenticates inventory and DNS/artifact inputs and commits with CAS.
mod matching;
mod paths;
mod transports;
use crate::{execution::*, *};
pub use matching::profile_allowed;
use meshway_model::{Digest, NetworkId, NodeId, PublicationId, RequestId, Revision};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub const COMPILER_VERSION: &str = "meshway-demand-1";
pub const MAX_DEMANDS: usize = 256;
pub const MAX_PATHS: usize = 4;
pub const MAX_HOPS: usize = 8;

/// No online flag: lack of contact cannot remove mandatory hops or grant direct access.
pub struct CompilationInput<'a> {
    pub network: NetworkId,
    pub revision: Revision,
    pub publication: PublicationId,
    pub configuration: &'a NetworkConfiguration,
    pub inventory: &'a BTreeMap<NodeId, TrustedNode>,
    pub demands: &'a BTreeMap<RequestId, Demand>,
    pub dns: &'a [TrustedDnsAssociation],
    pub rule_sets: &'a BTreeMap<String, RuleSetContent>,
    pub selected_profiles: &'a BTreeMap<NodeId, String>,
    pub now_ms: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CompileError {
    #[error("compilation budget exceeded")]
    Budget,
    #[error("inconsistent trusted compilation input")]
    InvalidInput,
    #[error("complete transport material is missing or incompatible")]
    Material,
    #[error("compilation serialization failed")]
    Serialization,
}

pub fn digest<T: Serialize>(value: &T) -> Result<Digest, CompileError> {
    let bytes = serde_json::to_vec(value).map_err(|_| CompileError::Serialization)?;
    Ok(Digest::from_bytes(Sha256::digest(bytes).into()))
}

pub fn compile(input: CompilationInput<'_>) -> Result<CompiledPublication, CompileError> {
    if input.demands.len() > MAX_DEMANDS
        || input.configuration.nodes.len() > 10_000
        || input.dns.len() > 4096
    {
        return Err(CompileError::Budget);
    }
    let mut source_profiles = BTreeMap::new();
    if input
        .dns
        .iter()
        .any(|a| a.network_id != input.network || a.configuration_revision != input.revision)
        || input.rule_sets.values().any(|r| {
            r.network_id != input.network
                || r.ip_cidr.iter().any(|s| s.parse::<ipnet::IpNet>().is_err())
        })
    {
        return Err(CompileError::InvalidInput);
    }
    for (id, d) in input.demands {
        if id != &d.id || (d.protocol == BusinessProtocol::IcmpEcho) != d.port.is_none() {
            return Err(CompileError::InvalidInput);
        }
        if d.expires_at_ms > input.now_ms
            && source_profiles
                .insert(d.source, &d.profile)
                .is_some_and(|p| p != &d.profile)
        {
            return Err(CompileError::InvalidInput);
        }
    }
    let dependency_digest = digest(&(
        COMPILER_VERSION,
        input.network,
        input.revision,
        input.configuration,
        input.inventory,
        input.demands,
        input.dns,
        input.rule_sets,
        input.selected_profiles,
    ))?;
    let mut nodes = BTreeMap::new();
    // Every trusted configured node gets a full target, including empty/disabled
    // targets. Removing the last demand cannot leave an old permit installed.
    for (id, trusted) in input.inventory {
        let Some(settings) = input.configuration.nodes.get(id) else {
            continue;
        };
        if trusted.control_key.public_key.algorithm != meshway_model::KeyAlgorithm::Ed25519
            || trusted.dataplane_key.public_key.algorithm != meshway_model::KeyAlgorithm::X25519
        {
            return Err(CompileError::InvalidInput);
        }
        nodes.insert(
            *id,
            NodeConfiguration {
                network_id: input.network,
                node_id: *id,
                revision: input.revision,
                publication: input.publication,
                dependency_digest,
                enabled: settings.enabled,
                hostname: settings.hostname.clone(),
                identity: settings.identity.clone(),
                addresses: settings.addresses.clone(),
                self_keys: trusted.clone(),
                listeners: if settings.enabled {
                    settings.listeners.clone()
                } else {
                    BTreeMap::new()
                },
                dialers: BTreeMap::new(),
                peers: BTreeMap::new(),
                permits: BTreeMap::new(),
                decisions: BTreeMap::new(),
                dns: DnsExecution {
                    settings: input.configuration.dns.clone(),
                    mesh_hosts: BTreeMap::new(),
                    associations: Vec::new(),
                    profile: selected_profile(&input, *id)?,
                },
                exit_enabled: settings.enabled && settings.exit_enabled,
            },
        );
    }
    let graph = paths::Graph::new(&input)?;
    for demand in input
        .demands
        .values()
        .filter(|d| d.expires_at_ms > input.now_ms)
    {
        let decision = evaluate(&input, &graph, demand)?;
        if let DemandDecision::Permitted { permit } = &decision {
            for path in &permit.paths {
                for id in &path.nodes {
                    let node = nodes.get_mut(id).ok_or(CompileError::InvalidInput)?;
                    node.permits.insert(demand.id, permit.clone());
                    for peer in &path.nodes {
                        if peer == id {
                            continue;
                        }
                        let s = &input.configuration.nodes[peer];
                        let t = &input.inventory[peer];
                        node.peers.insert(
                            *peer,
                            PeerConfiguration {
                                identity: s.identity.clone(),
                                addresses: s.addresses.clone(),
                                control_key: t.control_key.clone(),
                                dataplane_key: t.dataplane_key.clone(),
                            },
                        );
                        node.dns.mesh_hosts.insert(
                            format!("{}.{}", s.hostname, input.configuration.dns.domain),
                            s.addresses.clone(),
                        );
                    }
                    if let Some(keys) = path.dial_candidates.get(id) {
                        for key in keys {
                            node.dialers
                                .insert(key.clone(), graph.candidates.borrow()[id][key].clone());
                        }
                    }
                }
            }
        }
        if let Some(node) = nodes.get_mut(&demand.source) {
            node.decisions.insert(demand.id, decision);
            if let BusinessTarget::Domain { name, .. } = &demand.target {
                node.dns.associations.extend(
                    input
                        .dns
                        .iter()
                        .filter(|a| {
                            a.source == demand.source
                                && a.name == *name
                                && a.expires_at_ms > input.now_ms
                        })
                        .cloned(),
                );
            }
        }
    }
    Ok(CompiledPublication {
        network_id: input.network,
        revision: input.revision,
        publication: input.publication,
        dependency_digest,
        compiler_version: COMPILER_VERSION.into(),
        nodes: nodes
            .into_iter()
            .map(|(id, configuration)| {
                Ok((
                    id,
                    CompiledNode {
                        digest: digest(&configuration)?,
                        configuration,
                    },
                ))
            })
            .collect::<Result<_, CompileError>>()?,
    })
}

fn evaluate(
    input: &CompilationInput<'_>,
    graph: &paths::Graph,
    demand: &Demand,
) -> Result<DemandDecision, CompileError> {
    let unavailable = |reason| Ok(DemandDecision::Unavailable { reason });
    let cfg = input.configuration;
    if !cfg.nodes.get(&demand.source).is_some_and(|n| n.enabled)
        || !input.inventory.contains_key(&demand.source)
    {
        return unavailable(UnavailableReason::InactiveNode);
    }
    if !matching::profile_allowed(cfg, demand.source, &demand.profile)?
        || demand.profile != selected_profile(input, demand.source)?
    {
        return unavailable(UnavailableReason::InvalidProfile);
    }
    if let BusinessTarget::Domain { name, address } = &demand.target
        && !input.dns.iter().any(|a| {
            a.source == demand.source
                && a.name == *name
                && a.addresses.contains(address)
                && a.expires_at_ms > input.now_ms
        })
    {
        return unavailable(UnavailableReason::MissingDns);
    }
    let (targets, via) = match &demand.target {
        BusinessTarget::Mesh { node } => {
            let via = cfg
                .routing
                .mesh
                .iter()
                .find(|r| {
                    matching::selectors(cfg, &r.src, demand.source)
                        && matching::selectors(cfg, &r.dst, *node)
                })
                .map(|r| r.via.clone())
                .unwrap_or_default();
            (vec![*node], via)
        }
        _ => match matching::egress_action(input, demand)? {
            RouteAction::Direct => return Ok(DemandDecision::Direct),
            RouteAction::Unmanaged => return Ok(DemandDecision::Unmanaged),
            RouteAction::Exit { nodes, via } => {
                let targets = nodes
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?;
                (targets, via)
            }
        },
    };
    if !matching::access(input, demand) {
        return unavailable(UnavailableReason::Unauthorized);
    }
    let via = via
        .iter()
        .map(|s| matching::node_selector(s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    for target in targets {
        if !cfg.nodes.get(&target).is_some_and(|n| {
            n.enabled && (matches!(demand.target, BusinessTarget::Mesh { .. }) || n.exit_enabled)
        }) {
            continue;
        }
        let available = graph.paths(demand.source, target, &via)?;
        for path in available {
            candidates.push(path);
            if candidates.len() == MAX_PATHS {
                break;
            }
        }
        if candidates.len() == MAX_PATHS {
            break;
        }
    }
    if candidates.is_empty() {
        return unavailable(UnavailableReason::NoPath);
    }
    let expires_at_ms = input.dns.iter().filter(|a| matches!(&demand.target, BusinessTarget::Domain { name, .. } if a.source == demand.source && &a.name == name))
        .fold(demand.expires_at_ms, |limit, a| limit.min(a.expires_at_ms));
    Ok(DemandDecision::Permitted {
        permit: PathPermit {
            demand_id: demand.id,
            source: demand.source,
            source_addresses: cfg.nodes[&demand.source].addresses.clone(),
            target: demand.target.clone(),
            protocol: demand.protocol,
            port: demand.port,
            profile: demand.profile.clone(),
            paths: candidates,
            expires_at_ms,
            replies: ReplyPolicy::StatefulExactReversePath,
        },
    })
}
fn selected_profile(input: &CompilationInput<'_>, node: NodeId) -> Result<String, CompileError> {
    match input.selected_profiles.get(&node) {
        Some(profile) if matching::profile_allowed(input.configuration, node, profile)? => {
            Ok(profile.clone())
        }
        _ => matching::default_profile(input.configuration, node),
    }
}
