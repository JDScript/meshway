//! Full grants before distribution: one per (source, Mesh target) and one per
//! (source, exit Node, destination scope, waypoints).
use super::*;
use crate::compile::routing::PathError;
use ipnet::IpNet;
use meshway_configuration::execution::ExitDestinationScope;
use meshway_model::exit::ExitFamily;

/// Mirrors per-demand path search: cycles, limits and missing carriers yield
/// no grant for this pair; other compile errors abort the whole compilation.
fn search(
    graph: &paths::Graph,
    source: NodeId,
    target: NodeId,
    via: Option<&[NodeId]>,
) -> Result<Vec<LogicalPath>, CompileError> {
    match graph.paths(source, target, via, MAX_PATHS) {
        Ok(paths) => Ok(paths),
        Err(PathError::Cycle | PathError::Limit) => Ok(Vec::new()),
        Err(PathError::Compile(CompileError::EdgeTransportUnavailable { .. })) => Ok(Vec::new()),
        Err(PathError::Compile(error)) => Err(error),
    }
}

fn enabled(input: &NetmapInput<'_>, node: NodeId) -> bool {
    input
        .configuration
        .nodes
        .get(&node)
        .is_some_and(|n| n.enabled)
        && input.inventory.contains_key(&node)
}

pub(super) fn mesh(
    input: &NetmapInput<'_>,
    graph: &paths::Graph,
    members: &[NodeId],
) -> Result<Vec<Grant>, CompileError> {
    let cfg = input.configuration;
    let mut grants = Vec::new();
    for &source in members.iter().filter(|id| enabled(input, **id)) {
        for &target in members.iter().filter(|id| **id != source) {
            if !cfg.nodes[&target].enabled {
                continue;
            }
            let services = crate::access::mesh_services(cfg, input.subjects, source, target);
            if services.is_empty() {
                continue;
            }
            let paths = search(graph, source, target, None)?;
            if paths.is_empty() {
                continue;
            }
            grants.push(Grant {
                source,
                source_addresses: cfg.nodes[&source].addresses.clone(),
                target: GrantTarget::Mesh { node: target },
                services: Some(services),
                exit: None,
                paths: paths.into_iter().map(|p| input.grant_path(p)).collect(),
                replies: ReplyPolicy::StatefulExactReversePath,
            });
        }
    }
    Ok(grants)
}

/// External Access scopes of one source with the union of their services.
/// Domain selectors need trusted DNS context and IPv6 exits are unsupported.
fn scopes(
    input: &NetmapInput<'_>,
    source: NodeId,
) -> Vec<(ExitDestinationScope, IpScope, ServiceSet)> {
    let cfg = input.configuration;
    let mut grouped: BTreeMap<String, Vec<ServiceEntry>> = BTreeMap::new();
    for rule in cfg
        .access
        .iter()
        .filter(|rule| matching::selectors(cfg, input.subjects, &rule.src, source))
    {
        for selector in &rule.dst {
            if selector == "autogroup:internet"
                || selector
                    .parse::<IpNet>()
                    .is_ok_and(|net| net.addr().is_ipv4())
            {
                grouped
                    .entry(selector.clone())
                    .or_default()
                    .extend(crate::access::token_services(&rule.ip));
            }
        }
    }
    grouped
        .into_iter()
        .filter_map(|(selector, services)| {
            let services = ServiceSet::from_grants(services);
            if services.is_empty() {
                return None;
            }
            Some(if selector == "autogroup:internet" {
                (ExitDestinationScope::Internet, IpScope::Internet, services)
            } else {
                let net = selector.parse().ok()?;
                (
                    ExitDestinationScope::Cidr { prefix: selector },
                    IpScope::Net(net),
                    services,
                )
            })
        })
        .collect()
}

type ServiceEntry = (
    meshway_configuration::execution::BusinessProtocol,
    Option<(u16, u16)>,
);

#[derive(Clone, Copy)]
enum IpScope {
    Internet,
    Net(IpNet),
}

impl IpScope {
    fn overlaps(self, route: &IpNet) -> bool {
        let overlap = |net: &IpNet| net.contains(route) || route.contains(net);
        match self {
            Self::Internet => crate::internet_ranges().iter().any(overlap),
            Self::Net(net) => overlap(&net),
        }
    }
}

pub(super) fn scope_id(
    destination: &ExitDestinationScope,
    via: &[NodeId],
) -> Result<ExitScopeId, CompileError> {
    Ok(ExitScopeId(digest(&(
        "meshway-netmap-exit-scope",
        destination,
        via,
    ))?))
}

pub(super) fn exit(
    input: &NetmapInput<'_>,
    graph: &paths::Graph,
    members: &[NodeId],
    locals: &BTreeMap<NodeId, local::Local>,
) -> Result<Vec<Grant>, CompileError> {
    let cfg = input.configuration;
    let mut grants = Vec::new();
    for &source in members.iter().filter(|id| enabled(input, **id)) {
        let settings = &cfg.nodes[&source];
        let plan = &locals[&source].egress;
        if settings.addresses.ipv4.is_none() || plan.capture.unavailable.is_some() {
            continue;
        }
        // (exit, via) -> IPv4 prefixes whose first match selects that exit.
        let mut groups: BTreeMap<(NodeId, Vec<NodeId>), Vec<IpNet>> = BTreeMap::new();
        for route in &plan.routes {
            let EgressAction::Exit { nodes, via } = &route.action else {
                continue;
            };
            let net: IpNet = route
                .prefix
                .parse()
                .map_err(|_| CompileError::InvalidInput)?;
            if !net.addr().is_ipv4() {
                continue;
            }
            if nodes.len() > MAX_EXIT_CANDIDATES {
                return Err(CompileError::Budget);
            }
            for node in nodes {
                groups.entry((*node, via.clone())).or_default().push(net);
            }
        }
        if groups.is_empty() {
            continue;
        }
        let scopes = scopes(input, source);
        if scopes.is_empty() {
            continue;
        }
        for ((exit, via), mut routes) in groups {
            if exit == source
                || !cfg
                    .nodes
                    .get(&exit)
                    .is_some_and(|n| n.enabled && n.exit_enabled)
                || locals.get(&exit).is_none_or(|l| l.exit_role.is_none())
            {
                continue;
            }
            let paths = search(graph, source, exit, Some(&via))?;
            if paths.is_empty() {
                continue;
            }
            routes.sort();
            let paths: Vec<GrantPath> = paths.into_iter().map(|p| input.grant_path(p)).collect();
            for (destination, scope, services) in &scopes {
                if !routes.iter().any(|route| scope.overlaps(route)) {
                    continue;
                }
                grants.push(Grant {
                    source,
                    source_addresses: settings.addresses.clone(),
                    target: GrantTarget::Exit {
                        node: exit,
                        scope: scope_id(destination, &via)?,
                    },
                    services: Some(services.clone()),
                    exit: Some(GrantExit {
                        destination: destination.clone(),
                        routes: routes.iter().map(ToString::to_string).collect(),
                        family: ExitFamily::Ipv4,
                        selection: plan.capture.generation,
                    }),
                    paths: paths.clone(),
                    replies: ReplyPolicy::StatefulExactReversePath,
                });
            }
        }
    }
    Ok(grants)
}
