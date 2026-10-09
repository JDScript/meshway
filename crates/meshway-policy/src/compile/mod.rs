//! Pure bounded demand compilation from one trusted input snapshot.
//! The caller authenticates inventory and DNS/artifact inputs and commits with CAS.
mod capture;
mod diagnostics;
mod dns;
mod exit;
pub use capture::{MAX_CAPTURE_PREFIXES, egress_capture_plan};
pub use exit::ExitInputs;
mod edge_constraints;
mod matching;
pub mod netmap;
mod paths;
mod plan;
mod routing;
mod transports;
use crate::{SubjectContext, domain_name_matches, domain_suffix_matches, topology_allows};
pub(crate) use edge_constraints::validate_edge_transports;
pub use matching::profile_allowed;
use meshway_configuration::{execution::*, *};
use meshway_model::{NetworkId, NodeId, PublicationId, RequestId, Revision};
pub use plan::{PlanDestination, PlanRequest, plan, plan_with_exit};
use std::collections::BTreeMap;

pub const COMPILER_VERSION: &str = "meshway-demand-7";
pub const MAX_DEMANDS: usize = 256;
pub const MAX_PATHS: usize = 4;
/// Ordered exit candidates have a separate budget from alternatives per exit.
pub const MAX_EXIT_CANDIDATES: usize = 16;
pub const MAX_HOPS: usize = 8;

/// The declared graph with its trusted inventory: everything path search and
/// transport selection consult. Demands, DNS and rule-set content stay out.
#[derive(Clone, Copy)]
pub struct Topology<'a> {
    pub configuration: &'a NetworkConfiguration,
    pub subjects: &'a SubjectContext,
    pub inventory: &'a BTreeMap<NodeId, TrustedNode>,
}

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
    #[error(
        "/topology/{rule}/transports: edge {a}--{b} has no allowed transport in either dial direction"
    )]
    EdgeTransportUnavailable { rule: usize, a: NodeId, b: NodeId },
    #[error("execution dependency is unavailable")]
    UnavailableDependency,
    #[error("compilation serialization failed")]
    Serialization,
}

impl From<meshway_configuration::SerializationError> for CompileError {
    fn from(_: meshway_configuration::SerializationError) -> Self {
        Self::Serialization
    }
}

impl CompilationInput<'_> {
    pub(super) fn topology(&self) -> Topology<'_> {
        Topology {
            configuration: self.configuration,
            subjects: self.subjects,
            inventory: self.inventory,
        }
    }
}

pub fn compile(input: CompilationInput<'_>) -> Result<CompiledPublication, CompileError> {
    compile_with_diagnostics(input, &BTreeMap::new())
}

pub fn compile_with_diagnostics(
    input: CompilationInput<'_>,
    diagnostics: &BTreeMap<meshway_model::ObservationId, DiagnosticDemand>,
) -> Result<CompiledPublication, CompileError> {
    compile_with_exit(input, diagnostics, &ExitInputs::default())
}

pub fn compile_with_exit(
    input: CompilationInput<'_>,
    diagnostics: &BTreeMap<meshway_model::ObservationId, DiagnosticDemand>,
    runtime: &ExitInputs,
) -> Result<CompiledPublication, CompileError> {
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
    // Source instance renewal affects only DNS plans that actually depend on
    // it. Ordinary no-exit targets must not churn on unrelated lease renewal.
    let mut dns_source_deadlines = BTreeMap::new();
    for (&node, &expiry) in &runtime.instance_expires_at_ms {
        if input.inventory.contains_key(&node)
            && input.configuration.nodes.contains_key(&node)
            && !matches!(
                matching::dns_egress(input.configuration, &selected_profile(&input, node)?),
                DnsEgress::NoExit
            )
        {
            dns_source_deadlines.insert(node, expiry);
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
        diagnostics,
        &runtime.instances,
        &dns_source_deadlines,
        &runtime.ready_until_ms,
        &runtime.selection_generations,
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
        let profile = selected_profile(&input, *id)?;
        let dns_egress = matching::dns_egress(input.configuration, &profile);
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
                mesh_address_pools: input.configuration.address_pools.clone(),
                egress_capture: egress_capture_plan(
                    &input,
                    *id,
                    runtime.selection_generations.get(id).copied().unwrap_or(0),
                )?,
                exit_role: exit::role(&input, runtime, *id)?,
                self_keys: trusted.clone(),
                listeners: if settings.enabled {
                    settings.listeners.clone()
                } else {
                    BTreeMap::new()
                },
                dialers: BTreeMap::new(),
                peers: BTreeMap::new(),
                permits: BTreeMap::new(),
                local_egress: BTreeMap::new(),
                diagnostics: BTreeMap::new(),
                decisions: BTreeMap::new(),
                dns: DnsExecution {
                    settings: input.configuration.dns.clone(),
                    mesh_hosts: BTreeMap::new(),
                    associations: Vec::new(),
                    profile,
                    egress: dns_egress,
                },
                exit_enabled: settings.enabled && settings.exit_enabled,
            },
        );
    }
    let graph = paths::Graph::new(input.topology())?;
    for demand in input
        .demands
        .values()
        .filter(|d| d.expires_at_ms > input.now_ms)
    {
        let (decision, local_egress) =
            evaluate_with_local(&input, &graph, demand, exit::Facts::Runtime(runtime))?;
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
                        // Membership in a forwarding path is not permission
                        // to initiate business traffic to that peer. Published
                        // names obey the same source-scoped Access predicate as
                        // exact-name discovery, including relay/receiver views.
                        if crate::mesh_access_allows_any(
                            input.configuration,
                            input.subjects,
                            *id,
                            *peer,
                        ) {
                            node.dns.mesh_hosts.insert(
                                node_dns_name(&s.hostname, &input.configuration.dns.domain)
                                    .map_err(|_| CompileError::InvalidInput)?,
                                s.addresses.clone(),
                            );
                        }
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
            if let Some(permit) = local_egress {
                node.local_egress.insert(demand.id, permit);
            }
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
    diagnostics::install(&input, &graph, diagnostics, &mut nodes)?;
    dns::install(&input, &graph, runtime, &mut nodes)?;
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
    evaluate_with_local(
        input,
        graph,
        demand,
        exit::Facts::Runtime(&ExitInputs::default()),
    )
    .map(|(decision, _)| decision)
}

fn evaluate_with_local(
    input: &CompilationInput<'_>,
    graph: &paths::Graph,
    demand: &Demand,
    facts: exit::Facts<'_>,
) -> Result<(DemandDecision, Option<LocalEgressPermit>), CompileError> {
    let unavailable = |reason| Ok((DemandDecision::Unavailable { reason }, None));
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
        // IPv4-mapped IPv6 must not escape managed IPv4 capture/authorization.
        // Only classification is canonicalized; external local bindings retain
        // the exact requested target for endpoint verification.
        let address = address.0.to_canonical();
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
    let capture = egress_capture_plan(input, demand.source, facts.selection(demand.source))?;
    if !matches!(demand.target, BusinessTarget::Mesh { .. })
        && let Some(reason) = capture.unavailable
    {
        return unavailable(reason);
    }
    let local = |action, routing_expiry| {
        Ok((
            match action {
                LocalEgressAction::Direct => DemandDecision::Direct,
                LocalEgressAction::Unmanaged => DemandDecision::Unmanaged,
            },
            Some(LocalEgressPermit {
                demand_id: demand.id,
                source: demand.source,
                target: demand.target.clone(),
                protocol: demand.protocol,
                port: demand.port,
                profile: demand.profile.clone(),
                action,
                expires_at_ms: demand.expires_at_ms.min(dns_expiry).min(routing_expiry),
            }),
        ))
    };
    let (targets, via, routing_expiry) = match &demand.target {
        BusinessTarget::Mesh { node } => (vec![*node], None, demand.expires_at_ms),
        _ => match match matching::egress_action(input, demand) {
            Ok(action) => action,
            Err(CompileError::UnavailableDependency) => {
                return unavailable(UnavailableReason::MissingDependency);
            }
            Err(error) => return Err(error),
        } {
            (RouteAction::Direct, expires) => return local(LocalEgressAction::Direct, expires),
            (RouteAction::Unmanaged, expires) => {
                return local(LocalEgressAction::Unmanaged, expires);
            }
            (RouteAction::Exit { nodes, via }, expires) => {
                let targets = nodes
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<Vec<_>, _>>()?;
                (targets, Some(via), expires)
            }
        },
    };
    let Some(access_expiry) = matching::access_expiry(input, demand) else {
        return unavailable(UnavailableReason::Unauthorized);
    };
    let external = !matches!(demand.target, BusinessTarget::Mesh { .. });
    if external {
        let address = match demand.target {
            BusinessTarget::Ip { address } | BusinessTarget::Domain { address, .. } => address.0,
            _ => unreachable!(),
        };
        if address.is_ipv6() || cfg.nodes[&demand.source].addresses.ipv4.is_none() {
            return unavailable(UnavailableReason::UnsupportedFamily);
        }
        if !crate::is_exit_destination(address) {
            return unavailable(UnavailableReason::Unauthorized);
        }
    }
    let via = via
        .map(|via| {
            via.iter()
                .map(|s| matching::node_selector(s))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let mut candidates = Vec::new();
    let mut roles = BTreeMap::new();
    let mut capability_expiry = u64::MAX;
    let mut reason = UnavailableReason::NoPath;
    if targets.len() > MAX_EXIT_CANDIDATES {
        return Err(CompileError::Budget);
    }
    for target in targets {
        if external && target == demand.source {
            continue;
        }
        if !cfg.nodes.get(&target).is_some_and(|n| {
            n.enabled && (matches!(demand.target, BusinessTarget::Mesh { .. }) || n.exit_enabled)
        }) {
            continue;
        }
        if external {
            match facts {
                exit::Facts::Runtime(runtime) => {
                    let role = exit::role(input, runtime, target)?;
                    if !exit::available(input.topology(), runtime, target, input.now_ms)
                        || role.is_none()
                    {
                        reason = UnavailableReason::ExitUnavailable;
                        continue;
                    }
                    if let Some(role) = role {
                        roles.insert(target, role);
                    }
                }
                // Readiness is a dynamic fact: only the stable capability counts.
                exit::Facts::Stable(_) => {
                    if !exit::capable(input.topology(), target) {
                        reason = UnavailableReason::ExitUnavailable;
                        continue;
                    }
                }
            }
        }
        // Each ordered exit owns a bounded search. An invalid route produces
        // no permission, without blocking revocation/publication for other demands.
        match graph.paths(demand.source, target, via.as_deref(), MAX_PATHS) {
            Ok(paths) => {
                if external
                    && !paths.is_empty()
                    && let exit::Facts::Runtime(runtime) = facts
                {
                    capability_expiry = capability_expiry.min(runtime.ready_until_ms[&target]);
                }
                candidates.extend(paths);
            }
            Err(routing::PathError::Cycle) => reason = UnavailableReason::RoutingCycle,
            Err(routing::PathError::Limit) => reason = UnavailableReason::RoutingLimit,
            Err(routing::PathError::Compile(CompileError::EdgeTransportUnavailable { .. })) => {}
            Err(routing::PathError::Compile(error)) => return Err(error),
        }
    }
    if candidates.is_empty() {
        return unavailable(reason);
    }
    let expires_at_ms = dns_expiry
        .min(access_expiry)
        .min(routing_expiry)
        .min(capability_expiry);
    let exit = if external {
        let Some(destination) = exit::destination(input, demand) else {
            return unavailable(UnavailableReason::Unauthorized);
        };
        roles.retain(|node, _| {
            candidates
                .iter()
                .any(|path| path.nodes.last() == Some(node))
        });
        Some(ExitPermit {
            generation: capture.generation,
            family: meshway_model::exit::ExitFamily::Ipv4,
            destination,
            roles,
        })
    } else {
        None
    };
    Ok((
        DemandDecision::Permitted {
            permit: PathPermit {
                demand_id: demand.id,
                source: demand.source,
                source_addresses: cfg.nodes[&demand.source].addresses.clone(),
                target: demand.target.clone(),
                protocol: demand.protocol,
                port: demand.port,
                profile: demand.profile.clone(),
                exit,
                paths: candidates,
                expires_at_ms,
                replies: ReplyPolicy::StatefulExactReversePath,
            },
        },
        None,
    ))
}
fn selected_profile(input: &CompilationInput<'_>, node: NodeId) -> Result<String, CompileError> {
    effective_profile(
        input.configuration,
        input.subjects,
        input.selected_profiles,
        node,
    )
}
fn effective_profile(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    selected: &BTreeMap<NodeId, String>,
    node: NodeId,
) -> Result<String, CompileError> {
    match selected.get(&node) {
        Some(profile) if matching::profile_allowed(configuration, subjects, node, profile)? => {
            Ok(profile.clone())
        }
        _ => matching::default_profile(configuration, subjects, node),
    }
}
