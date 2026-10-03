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
/// Ordered exit candidates have a separate budget from alternatives per exit.
pub const MAX_EXIT_CANDIDATES: usize = 16;
pub const MAX_HOPS: usize = 8;

/// No online flag: lack of contact cannot remove mandatory hops or grant direct access.
pub struct CompilationInput<'a> {
    pub network: NetworkId,
    pub revision: Revision,
    pub publication: PublicationId,
    pub configuration: &'a NetworkConfiguration,
    pub subjects: &'a SubjectContext,
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
    #[error("execution dependency is unavailable")]
    UnavailableDependency,
    #[error("compilation serialization failed")]
    Serialization,
}

pub fn digest<T: Serialize>(value: &T) -> Result<Digest, CompileError> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(value).map_err(|_| CompileError::Serialization)?,
    );
    Ok(Digest::from_bytes(Sha256::digest(bytes.as_slice()).into()))
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
        input.subjects,
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
        if !input.subjects.node_is_active(input.configuration, *id) {
            return Err(CompileError::InvalidInput);
        }
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
    if !matching::profile_allowed(cfg, input.subjects, demand.source, &demand.profile)?
        || demand.profile != selected_profile(input, demand.source)?
    {
        return unavailable(UnavailableReason::InvalidProfile);
    }
    let dns_expiry = if let BusinessTarget::Domain { name, address } = &demand.target {
        let Some(expires) = input
            .dns
            .iter()
            .filter(|a| {
                a.source == demand.source
                    && a.name == *name
                    && a.addresses.contains(address)
                    && a.expires_at_ms > input.now_ms
            })
            .map(|a| a.expires_at_ms)
            .max()
        else {
            return unavailable(UnavailableReason::MissingDns);
        };
        demand.expires_at_ms.min(expires)
    } else {
        demand.expires_at_ms
    };
    let mut normalized = demand.clone();
    if let BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } = &demand.target
    {
        let address = address.0;
        if let Some((node, _)) = cfg.nodes.iter().find(|(_, n)| {
            n.addresses.ipv4.map(std::net::IpAddr::V4) == Some(address)
                || n.addresses.ipv6.map(std::net::IpAddr::V6) == Some(address)
        }) {
            normalized.target = BusinessTarget::Mesh { node: *node };
        } else if cfg
            .address_pools
            .ipv4
            .iter()
            .chain(&cfg.address_pools.ipv6)
            .any(|p| {
                p.parse::<ipnet::IpNet>()
                    .is_ok_and(|p| p.contains(&address))
            })
        {
            // Unassigned Mesh addresses remain captured/rejected. They must
            // never fall through to a local direct/unmanaged egress fallback.
            return unavailable(UnavailableReason::NoPath);
        }
    }
    let demand = &normalized;
    let (targets, via, routing_expiry) = match &demand.target {
        BusinessTarget::Mesh { node } => {
            let via = cfg
                .routing
                .mesh
                .iter()
                .find(|r| {
                    matching::selectors(cfg, input.subjects, &r.src, demand.source)
                        && matching::selectors(cfg, input.subjects, &r.dst, *node)
                })
                .map(|r| r.via.clone())
                .unwrap_or_default();
            (vec![*node], via, demand.expires_at_ms)
        }
        _ => match match matching::egress_action(input, demand) {
            Ok(action) => action,
            Err(CompileError::UnavailableDependency) => {
                return unavailable(UnavailableReason::MissingDependency);
            }
            Err(error) => return Err(error),
        } {
            (RouteAction::Direct, _) => return Ok(DemandDecision::Direct),
            (RouteAction::Unmanaged, _) => return Ok(DemandDecision::Unmanaged),
            (RouteAction::Exit { nodes, via }, expires) => {
                let targets = nodes
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?;
                (targets, via, expires)
            }
        },
    };
    let Some(access_expiry) = matching::access_expiry(input, demand) else {
        return unavailable(UnavailableReason::Unauthorized);
    };
    let via = via
        .iter()
        .map(|s| matching::node_selector(s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    let mut exits = Vec::new();
    if targets.len() > MAX_EXIT_CANDIDATES {
        return Err(CompileError::Budget);
    }
    for target in targets {
        if !cfg.nodes.get(&target).is_some_and(|n| {
            n.enabled && (matches!(demand.target, BusinessTarget::Mesh { .. }) || n.exit_enabled)
        }) {
            continue;
        }
        // Cover ordered exits first, before exploring additional routes to any
        // one of them. Each exit owns its alternative-path search allocation.
        if !graph.paths(demand.source, target, &via, 1)?.is_empty() {
            exits.push(target);
        }
    }
    for target in exits {
        // Keep exit priority in the installed result: X's alternatives precede Y.
        candidates.extend(graph.paths(demand.source, target, &via, MAX_PATHS)?);
    }
    if candidates.is_empty() {
        return unavailable(UnavailableReason::NoPath);
    }
    let expires_at_ms = dns_expiry.min(access_expiry).min(routing_expiry);
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
        Some(profile)
            if matching::profile_allowed(input.configuration, input.subjects, node, profile)? =>
        {
            Ok(profile.clone())
        }
        _ => matching::default_profile(input.configuration, input.subjects, node),
    }
}
