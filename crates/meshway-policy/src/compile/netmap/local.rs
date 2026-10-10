//! Per-node content outside the projection: egress partition, exit role,
//! names, resolver decisions and the DNS service plan.
use super::*;
use ipnet::IpNet;
use meshway_configuration::execution::{DnsEgressUnavailable, ExecutionAddress, ExitNat};
use meshway_model::exit::{EXIT_IPV4_FEATURE, EXIT_IPV4_VERSION, ExitFamily};
use std::net::IpAddr;

pub(super) struct Local {
    pub egress: EgressPlan,
    pub exit_role: Option<ExitRole>,
    pub dns: NetmapDns,
}

fn action(route: &RouteAction) -> Result<EgressAction, CompileError> {
    let nodes = |selectors: &[String]| {
        selectors
            .iter()
            .map(|s| matching::node_selector(s))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match route {
        RouteAction::Exit { nodes: exits, via } => EgressAction::Exit {
            nodes: nodes(exits)?,
            via: nodes(via)?,
        },
        RouteAction::Direct => EgressAction::Direct,
        RouteAction::Unmanaged => EgressAction::Unmanaged,
    })
}

/// The selection identity excludes the revision: an unrelated configuration
/// change keeps it, while any selection switch or routing change replaces it.
/// Shared by the Server and by Nodes recomputing a visible source's selection.
pub(super) fn egress(
    network: NetworkId,
    cfg: &NetworkConfiguration,
    rule_sets: &BTreeMap<String, RuleSetContent>,
    node: NodeId,
    profile: &str,
    selection: u64,
) -> Result<EgressPlan, CompileError> {
    let seed = digest(&(
        "meshway-netmap-egress",
        network,
        node,
        profile,
        selection,
        &cfg.routing.egress,
    ))?;
    let (mut capture, partition) = capture::capture(cfg, rule_sets, profile, seed)?;
    let mut routes = partition
        .into_iter()
        .map(|(prefix, route)| {
            Ok((
                prefix,
                EgressRoute {
                    prefix: prefix.to_string(),
                    action: action(&route)?,
                },
            ))
        })
        .collect::<Result<Vec<(IpNet, EgressRoute)>, CompileError>>()?;
    routes.sort_by_key(|(prefix, _)| *prefix);
    let routes: Vec<EgressRoute> = routes.into_iter().map(|(_, route)| route).collect();
    capture.generation = digest(&(capture.generation, &routes))?;
    Ok(EgressPlan { capture, routes })
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
fn resolvers(
    cfg: &NetworkConfiguration,
    enabled: bool,
    plan: &EgressPlan,
) -> BTreeMap<String, ResolverAction> {
    cfg.dns
        .servers
        .iter()
        .map(|(key, server)| {
            let address = server.server.to_canonical();
            let action = if !enabled
                || plan.capture.unavailable.is_some()
                || address.is_unspecified()
                || address.is_multicast()
                || mesh_address(cfg, address)
            {
                ResolverAction::Rejected
            } else {
                match plan.route(address).map(|route| &route.action) {
                    Some(EgressAction::Direct) => ResolverAction::Direct,
                    Some(EgressAction::Unmanaged) => ResolverAction::Unmanaged,
                    Some(EgressAction::Exit { .. }) => ResolverAction::Exit,
                    None => ResolverAction::Rejected,
                }
            };
            (key.clone(), action)
        })
        .collect()
}

impl Local {
    pub fn compile(input: &NetmapInput<'_>, node: NodeId) -> Result<Self, CompileError> {
        let cfg = input.configuration;
        let profile = input.profile(node)?;
        let egress = egress(
            input.network,
            cfg,
            input.rule_sets,
            node,
            &profile,
            input
                .selection_versions
                .get(&node)
                .map_or(0, |version| version.get()),
        )?;
        let enabled = cfg.nodes[&node].enabled;
        let plan = match matching::dns_egress(cfg, &profile) {
            DnsEgress::NoExit => DnsPlan::NoExit,
            _ => DnsPlan::Unavailable {
                reason: DnsEgressUnavailable::ExitPlanRequired,
            },
        };
        Ok(Self {
            exit_role: exit_role(input.network, cfg, input.inventory, node)?,
            dns: NetmapDns {
                mesh_hosts: mesh_hosts(input, node)?,
                resolvers: resolvers(cfg, enabled, &egress),
                profile,
                egress: plan,
            },
            egress,
        })
    }
}

/// Resolver services follow the same single-exit rule as before, without any
/// instance or readiness input. Returns every service for path distribution.
pub(super) fn dns(
    input: &NetmapInput<'_>,
    graph: &paths::Graph,
    locals: &mut BTreeMap<NodeId, Local>,
) -> Result<Vec<DnsService>, CompileError> {
    let mut services = Vec::new();
    let sources: Vec<NodeId> = locals.keys().copied().collect();
    for source in sources {
        if matches!(locals[&source].dns.egress, DnsPlan::NoExit) {
            continue;
        }
        let plan = match service(input, graph, locals, source)? {
            Ok(service) => {
                services.push(service.clone());
                DnsPlan::ViaExit {
                    service: Box::new(service),
                }
            }
            Err(reason) => DnsPlan::Unavailable { reason },
        };
        locals
            .get_mut(&source)
            .ok_or(CompileError::InvalidInput)?
            .dns
            .egress = plan;
    }
    for service in &services {
        let exit = locals
            .get_mut(&service.exit)
            .ok_or(CompileError::InvalidInput)?;
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
    graph: &paths::Graph,
    locals: &BTreeMap<NodeId, Local>,
    source: NodeId,
) -> Result<Result<DnsService, DnsEgressUnavailable>, CompileError> {
    let cfg = input.configuration;
    let local = &locals[&source];
    let Some((exit, via)) = super::super::dns::single_exit(cfg, &local.dns.profile)? else {
        return Ok(Err(DnsEgressUnavailable::ExitPlanRequired));
    };
    if exit == source
        || local.egress.capture.unavailable.is_some()
        || !matches!(
            matching::dns_egress(cfg, &input.profile(exit)?),
            DnsEgress::NoExit
        )
    {
        return Ok(Err(DnsEgressUnavailable::ExitPlanRequired));
    }
    let settings = &cfg.nodes[&source];
    if !settings.enabled || !super::super::dns::external_access(cfg, input.subjects, source) {
        return Ok(Err(DnsEgressUnavailable::Unauthorized));
    }
    let Some(role) = locals.get(&exit).and_then(|l| l.exit_role.clone()) else {
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
        generation: local.egress.capture.generation,
        role,
        path,
        transports: vec![DnsTransport::Udp, DnsTransport::Tcp],
        max_pending: 128,
        timeout_ms: 5_000,
    }))
}
