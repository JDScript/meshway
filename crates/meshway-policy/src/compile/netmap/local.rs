//! Per-node content outside the projection: egress partition, exit role,
//! names, resolver decisions and the DNS service plan.
use super::*;
use crate::compile::dns::DnsResolution;
use ipnet::IpNet;
use meshway_configuration::execution::{DnsEgressUnavailable, ExecutionAddress, ExitNat};
use meshway_model::exit::{EXIT_IPV4_FEATURE, EXIT_IPV4_VERSION, ExitFamily};
use std::net::IpAddr;

pub(super) struct Local {
    pub egress: EgressPlan,
    pub exit_role: Option<ExitRole>,
    pub dns: NetmapDns,
}

/// The source whose egress plan is built: its network, Node, effective
/// profile and selection version.
pub(super) struct Source<'a> {
    pub network: NetworkId,
    pub node: NodeId,
    pub profile: &'a str,
    pub selection: u64,
}

/// The selection identity excludes the revision: an unrelated configuration
/// change keeps it, while any selection switch, change of what `auto`
/// resolves to, routing change or new content of a set the rules use
/// replaces it. It does not depend on which artifacts
/// a Node holds, so every Node computes the same identity for a source.
/// Shared by the Server and by Nodes recomputing a visible source's selection.
pub(super) fn egress(
    source: Source<'_>,
    cfg: &NetworkConfiguration,
    exits: matching::Exits<'_>,
    references: &BTreeMap<String, meshway_rules::Digest>,
    partition: &capture::Partition,
) -> Result<EgressPlan, CompileError> {
    let Source {
        network,
        node,
        profile,
        selection,
    } = source;
    let capture::Selection { rules, fallback } = match capture::selection(cfg, profile, Some(node))
    {
        Ok(selected) => selected,
        Err(CompileError::Serialization) => return Err(CompileError::Serialization),
        Err(_) => capture::Selection {
            rules: Vec::new(),
            fallback: RouteAction::Unmanaged,
        },
    };
    let used: BTreeMap<&str, meshway_rules::Digest> = rules
        .iter()
        .flat_map(|(_, rule)| &rule.matcher.rule_sets)
        .filter_map(|key| references.get(key).map(|digest| (key.as_str(), *digest)))
        .collect();
    // What `auto` resolved to (profile, basis, region) is part of the
    // identity: a region change alone changes neither `routing.egress` nor
    // the selection version.
    let auto = matching::concrete_profile(cfg, profile, Some(node))
        .ok()
        .and_then(|(_, _, auto)| auto);
    let generation = digest(&(
        "meshway-netmap-egress",
        network,
        node,
        profile,
        auto,
        selection,
        &cfg.routing.egress,
        used,
    ))?;
    Ok(EgressPlan {
        generation,
        rules: rules
            .iter()
            .map(|(rule, egress)| {
                Ok(EgressRuleRef {
                    rule: rule.clone(),
                    action: capture::action(&matching::executed(
                        exits,
                        egress.action.clone().into(),
                        node,
                    )?)?,
                })
            })
            .collect::<Result<_, CompileError>>()?,
        fallback: capture::action(&matching::executed(exits, fallback, node)?)?,
        unavailable: partition.as_ref().err().copied(),
    })
}

pub(super) fn exit_role(
    network: NetworkId,
    cfg: &NetworkConfiguration,
    inventory: &BTreeMap<NodeId, TrustedNode>,
    node: NodeId,
) -> Result<Option<ExitRole>, CompileError> {
    let capable = inventory.get(&node).is_some_and(|trusted| {
        trusted.capabilities.get(EXIT_IPV4_FEATURE)
            == Some(&meshway_model::state::Capability::Feature {
                version: EXIT_IPV4_VERSION,
            })
    });
    if !capable
        || !cfg
            .nodes
            .get(&node)
            .is_some_and(|n| n.enabled && n.exit_enabled)
    {
        return Ok(None);
    }
    Ok(Some(ExitRole {
        generation: digest(&(
            "meshway-netmap-exit-role",
            network,
            node,
            ExitFamily::Ipv4,
            ExitNat::Masquerade,
        ))?,
        family: ExitFamily::Ipv4,
        nat: ExitNat::Masquerade,
    }))
}

/// Same predicate as exact-name discovery: the Node itself and every active
/// Node to which some Access rule grants it any service.
fn mesh_hosts(
    input: &NetmapInput<'_>,
    node: NodeId,
) -> Result<BTreeMap<String, NodeAddresses>, CompileError> {
    let cfg = input.configuration;
    let mut hosts = BTreeMap::new();
    if !cfg.dns.magic_dns {
        return Ok(hosts);
    }
    for (id, settings) in &cfg.nodes {
        if input.inventory.contains_key(id)
            && input.subjects.node_is_active(cfg, *id)
            && (*id == node || crate::mesh_access_allows_any(cfg, input.subjects, node, *id))
        {
            hosts.insert(
                node_dns_name(&settings.hostname, &cfg.dns.domain)
                    .map_err(|_| CompileError::InvalidInput)?,
                settings.addresses.clone(),
            );
        }
    }
    Ok(hosts)
}

fn mesh_address(cfg: &NetworkConfiguration, address: IpAddr) -> bool {
    cfg.nodes.values().any(|n| {
        n.addresses.ipv4.map(IpAddr::V4) == Some(address)
            || n.addresses.ipv6.map(IpAddr::V6) == Some(address)
    }) || cfg
        .address_pools
        .ipv4
        .iter()
        .chain(&cfg.address_pools.ipv6)
        .any(|pool| {
            pool.parse::<IpNet>()
                .is_ok_and(|pool| pool.contains(&address))
        })
}

/// The route of a configured resolver decides its local socket permission.
pub(super) fn resolvers(
    cfg: &NetworkConfiguration,
    enabled: bool,
    partition: &capture::Partition,
) -> BTreeMap<String, ResolverAction> {
    cfg.dns
        .servers
        .iter()
        .map(|(key, server)| {
            let address = server.server.to_canonical();
            let action = match partition {
                Ok(partition)
                    if enabled
                        && !address.is_unspecified()
                        && !address.is_multicast()
                        && !mesh_address(cfg, address) =>
                {
                    match partition.action(address) {
                        EgressAction::Direct => ResolverAction::Direct,
                        EgressAction::Unmanaged => ResolverAction::Unmanaged,
                        EgressAction::Exit { .. } => ResolverAction::Exit,
                    }
                }
                _ => ResolverAction::Rejected,
            };
            (key.clone(), action)
        })
        .collect()
}

impl Local {
    /// `exits` is [`matching::Exits::Full`] on the Server and the
    /// projection's list on a Node.
    pub fn compile(
        input: &NetmapInput<'_>,
        exits: matching::Exits<'_>,
        node: NodeId,
    ) -> Result<Self, CompileError> {
        let cfg = input.configuration;
        let profile = input.profile(node)?;
        let partition = capture::partition(cfg, exits, input.rule_sets, &profile, Some(node))
            .map(std::sync::Arc::new);
        let references = input
            .rule_sets
            .iter()
            .map(|(key, set)| (key.clone(), set.digest()))
            .collect();
        let egress = egress(
            Source {
                network: input.network,
                node,
                profile: &profile,
                selection: input
                    .selection_versions
                    .get(&node)
                    .map_or(0, |version| version.get()),
            },
            cfg,
            exits,
            &references,
            &partition,
        )?;
        let enabled = cfg.nodes[&node].enabled;
        // An exit resolution is completed by [`dns`] into its service.
        let plan = match super::super::dns::resolution(cfg, exits, input.rule_sets, &profile, node)?
        {
            DnsResolution::Local => DnsPlan::NoExit,
            _ => DnsPlan::Unavailable {
                reason: DnsEgressUnavailable::ExitPlanRequired,
            },
        };
        Ok(Self {
            exit_role: exit_role(input.network, cfg, input.inventory, node)?,
            dns: NetmapDns {
                mesh_hosts: mesh_hosts(input, node)?,
                resolvers: resolvers(cfg, enabled, &partition),
                profile,
                egress: plan,
            },
            egress,
        })
    }
}

/// Resolver services of every source whose names resolve through an exit
/// ([`crate::compile::dns::resolution`]), without any instance or readiness
/// input. Returns every service for path distribution.
pub(super) fn dns(
    input: &NetmapInput<'_>,
    graph: &paths::Graph,
    members: &[NodeId],
    locals: &mut BTreeMap<NodeId, Local>,
) -> Result<Vec<DnsService>, CompileError> {
    let mut services = Vec::new();
    for &source in members {
        let profile = input.profile(source)?;
        let resolution = super::super::dns::resolution(
            input.configuration,
            matching::Exits::Full(input.configuration),
            input.rule_sets,
            &profile,
            source,
        )?;
        if resolution == DnsResolution::Local
            || (!locals.contains_key(&source)
                && !matches!(&resolution, DnsResolution::Exit { exit, .. } if locals.contains_key(exit)))
        {
            continue;
        }
        let temporary;
        let local = match locals.get(&source) {
            Some(local) => local,
            None => {
                temporary =
                    Local::compile(input, matching::Exits::Full(input.configuration), source)?;
                &temporary
            }
        };
        let plan = match service(
            input,
            matching::Exits::Full(input.configuration),
            graph,
            local,
            source,
        )? {
            Ok(service) => {
                services.push(service.clone());
                DnsPlan::ViaExit {
                    service: Box::new(service),
                }
            }
            Err(reason) => DnsPlan::Unavailable { reason },
        };
        if let Some(local) = locals.get_mut(&source) {
            local.dns.egress = plan;
        }
    }
    for service in &services {
        let Some(exit) = locals.get_mut(&service.exit) else {
            continue;
        };
        match &mut exit.dns.egress {
            DnsPlan::NoExit => {
                exit.dns.egress = DnsPlan::ServeExitResolver {
                    services: vec![service.clone()],
                };
            }
            DnsPlan::ServeExitResolver { services } => services.push(service.clone()),
            _ => return Err(CompileError::InvalidInput),
        }
    }
    Ok(services)
}

pub(super) fn service(
    input: &NetmapInput<'_>,
    exits: matching::Exits<'_>,
    graph: &paths::Graph,
    local: &Local,
    source: NodeId,
) -> Result<Result<DnsService, DnsEgressUnavailable>, CompileError> {
    let cfg = input.configuration;
    let resolve = |profile: &str, node| {
        super::super::dns::resolution(cfg, exits, input.rule_sets, profile, node)
    };
    let DnsResolution::Exit { exit, via } = resolve(&local.dns.profile, source)? else {
        return Ok(Err(DnsEgressUnavailable::ExitPlanRequired));
    };
    // The exit resolves with its own resolvers, never through another exit.
    if local.egress.unavailable.is_some()
        || resolve(&input.profile(exit)?, exit)? != DnsResolution::Local
    {
        return Ok(Err(DnsEgressUnavailable::ExitPlanRequired));
    }
    let settings = &cfg.nodes[&source];
    if !settings.enabled || !super::super::dns::external_access(cfg, input.subjects, source) {
        return Ok(Err(DnsEgressUnavailable::Unauthorized));
    }
    let Some(role) = exit_role(input.network, cfg, input.inventory, exit)? else {
        return Ok(Err(DnsEgressUnavailable::ExitUnavailable));
    };
    let Some(address) = cfg.nodes.get(&exit).and_then(|n| n.addresses.ipv4) else {
        return Ok(Err(DnsEgressUnavailable::ExitUnavailable));
    };
    if settings.addresses.ipv4.is_none() {
        return Ok(Err(DnsEgressUnavailable::NoPath));
    }
    let path = match graph.paths(source, exit, Some(&via), 1) {
        Ok(paths) => paths.into_iter().next(),
        Err(routing::PathError::Compile(error @ CompileError::Serialization)) => return Err(error),
        Err(_) => None,
    };
    let Some(path) = path else {
        return Ok(Err(DnsEgressUnavailable::NoPath));
    };
    Ok(Ok(DnsService {
        source,
        source_addresses: settings.addresses.clone(),
        exit,
        address: ExecutionAddress(address.into()),
        profile: local.dns.profile.clone(),
        generation: local.egress.generation,
        role,
        path,
        transports: vec![DnsTransport::Udp, DnsTransport::Tcp],
        max_pending: 128,
        timeout_ms: 5_000,
    }))
}
