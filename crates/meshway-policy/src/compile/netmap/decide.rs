//! Node-side compilation over an installed netmap's projection: the same
//! evaluation as per-demand compilation, without dynamic exit facts.
use super::*;
use crate::compile::routing::PathError;
use meshway_configuration::execution::BusinessTarget;
use meshway_transport_spec::Port;
use std::{net::IpAddr, sync::Arc};

/// Business destination of a decision and the circuit unit relays check.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A Mesh Node; services and paths depend only on (source, node).
    Mesh { node: NodeId },
    /// Ordered primary/backup exits behind ordered waypoints.
    Exit {
        nodes: Vec<NodeId>,
        via: Vec<NodeId>,
    },
}

/// Executable result for one source flow; `paths` hold every allowed path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub source: NodeId,
    /// Exact destination the decision was computed for.
    pub destination: IpAddr,
    pub target: Target,
    /// Every service this source may use toward `destination`.
    pub services: ServiceSet,
    pub paths: Vec<LogicalPath>,
    /// Source egress selection identity; present for exit decisions.
    pub selection: Option<Digest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalDecision {
    Permitted(Decision),
    Direct,
    Unmanaged,
    Denied(UnavailableReason),
}

/// The projection converted once for repeated evaluation. Peer listeners and
/// dialers are absent from this configuration; carriers come from the
/// redacted declarations kept in the projection.
pub struct LocalCompiler {
    netmap: Arc<NodeNetmap>,
    configuration: NetworkConfiguration,
}

// Internal identifiers of the synthetic per-flow demand; never published.
const LOCAL_PUBLICATION: &str = "00000000-0000-4000-8000-00000000ffff";
const LOCAL_REQUEST: &str = "00000000-0000-4000-8000-00000000fffe";

fn rebuild(view: &ConfigurationView) -> NetworkConfiguration {
    NetworkConfiguration {
        address_pools: view.address_pools.clone(),
        defaults: Defaults {
            access: view.defaults.access,
            topology: view.defaults.topology,
            dialers: Default::default(),
        },
        groups: view.groups.clone(),
        tags_owners: view.tags_owners.clone(),
        access: view.access.clone(),
        nodes: view
            .nodes
            .iter()
            .map(|(id, n)| {
                (
                    *id,
                    NodeSettings {
                        hostname: n.hostname.clone(),
                        remark: n.remark.clone(),
                        identity: n.identity.clone(),
                        enabled: n.enabled,
                        addresses: n.addresses.clone(),
                        listeners: BTreeMap::new(),
                        dialers: Input::Omitted,
                        exit_enabled: n.exit_enabled,
                    },
                )
            })
            .collect(),
        dns: view.dns.clone(),
        topology: view.topology.clone(),
        routing: view.routing.clone(),
        rule_sets: view.rule_sets.clone(),
    }
}

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

fn selectors(selectors: &[String]) -> Result<Vec<NodeId>, CompileError> {
    selectors
        .iter()
        .map(|s| matching::node_selector(s))
        .collect()
}

impl LocalCompiler {
    pub fn new(netmap: &NodeNetmap) -> Self {
        Self::from_shared(Arc::new(netmap.clone()))
    }

    /// Own immutable content so a runtime may retain one compiler across
    /// requests. A compiler computes decisions; it never carries authority.
    pub fn from_shared(netmap: Arc<NodeNetmap>) -> Self {
        Self {
            configuration: rebuild(&netmap.projection.configuration),
            netmap,
        }
    }

    /// The same DNS service the Server compiled for `source`, reconstructed
    /// on this Node's projection. Relays use it to authenticate the complete
    /// DNS-purpose path without gaining permission to resolve the query.
    pub fn dns_service(&self, source: NodeId) -> Result<Option<DnsService>, CompileError> {
        if !self.configuration.nodes.contains_key(&source)
            || !self.netmap.projection.inventory.contains_key(&source)
        {
            return Ok(None);
        }
        let projection = &self.netmap.projection;
        let input = NetmapInput {
            network: self.netmap.network_id,
            revision: self.netmap.revision,
            configuration: &self.configuration,
            subjects: &projection.subjects,
            inventory: &projection.inventory,
            rule_sets: &projection.rule_sets,
            selected_profiles: &projection.selected_profiles,
            selection_versions: &projection.selection_versions,
            previous: &BTreeMap::new(),
        };
        let source_local = local::Local::compile(&input, source)?;
        let Some((exit, _)) =
            super::super::dns::single_exit(&self.configuration, &source_local.dns.profile)?
        else {
            return Ok(None);
        };
        if exit == source
            || !self.configuration.nodes.contains_key(&exit)
            || !projection.inventory.contains_key(&exit)
        {
            return Ok(None);
        }
        let locals = BTreeMap::from([
            (source, source_local),
            (exit, local::Local::compile(&input, exit)?),
        ]);
        let graph = paths::Graph::projected(input.topology(), &projection.configuration)?;
        Ok(local::service(&input, &graph, &locals, source)?.ok())
    }

    fn input(&self) -> Result<CompilationInput<'_>, CompileError> {
        let projection = &self.netmap.projection;
        static EMPTY: std::sync::OnceLock<BTreeMap<meshway_model::RequestId, Demand>> =
            std::sync::OnceLock::new();
        Ok(CompilationInput {
            network: self.netmap.network_id,
            revision: self.netmap.revision,
            publication: LOCAL_PUBLICATION
                .parse()
                .map_err(|_| CompileError::InvalidInput)?,
            configuration: &self.configuration,
            subjects: &projection.subjects,
            inventory: &projection.inventory,
            demands: EMPTY.get_or_init(BTreeMap::new),
            dns: &[],
            rule_sets: &projection.rule_sets,
            selected_profiles: &projection.selected_profiles,
            now_ms: 0,
        })
    }

    fn graph<'g>(
        &'g self,
        input: &'g CompilationInput<'g>,
    ) -> Result<paths::Graph<'g>, CompileError> {
        paths::Graph::projected(input.topology(), &self.netmap.projection.configuration)
    }

    /// Egress plan of a visible source, identical to the one its own netmap carries.
    pub fn egress(&self, source: NodeId) -> Result<EgressPlan, CompileError> {
        let projection = &self.netmap.projection;
        let profile = effective_profile(
            &self.configuration,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        local::egress(
            self.netmap.network_id,
            &self.configuration,
            &projection.rule_sets,
            source,
            &profile,
            projection
                .selection_versions
                .get(&source)
                .map_or(0, |version| version.get()),
        )
    }

    /// Decide one new flow of `source` toward `destination`.
    pub fn decide(
        &self,
        source: NodeId,
        destination: IpAddr,
        protocol: BusinessProtocol,
        port: Option<Port>,
    ) -> Result<LocalDecision, CompileError> {
        if (protocol == BusinessProtocol::IcmpEcho) != port.is_none() {
            return Ok(LocalDecision::Denied(UnavailableReason::Unauthorized));
        }
        if !self.configuration.nodes.contains_key(&source) {
            return Ok(LocalDecision::Denied(UnavailableReason::InactiveNode));
        }
        let input = self.input()?;
        let graph = self.graph(&input)?;
        let profile = effective_profile(
            &self.configuration,
            input.subjects,
            input.selected_profiles,
            source,
        )?;
        let demand = Demand {
            id: LOCAL_REQUEST
                .parse()
                .map_err(|_| CompileError::InvalidInput)?,
            source,
            target: BusinessTarget::Ip {
                address: ExecutionAddress(destination),
            },
            protocol,
            port,
            profile,
            expires_at_ms: u64::MAX,
        };
        let versions = &self.netmap.projection.selection_versions;
        let (decision, _) =
            evaluate_with_local(&input, &graph, &demand, exit::Facts::Stable(versions))?;
        let permit = match decision {
            DemandDecision::Permitted { permit } => permit,
            DemandDecision::Direct => return Ok(LocalDecision::Direct),
            DemandDecision::Unmanaged => return Ok(LocalDecision::Unmanaged),
            DemandDecision::Unavailable { reason } => return Ok(LocalDecision::Denied(reason)),
        };
        let cfg = &self.configuration;
        let (target, services, selection) = match permit.target {
            BusinessTarget::Mesh { node } => (
                Target::Mesh { node },
                crate::access::mesh_services(cfg, input.subjects, source, node),
                None,
            ),
            _ => {
                let normalized = Demand {
                    target: BusinessTarget::Ip {
                        address: ExecutionAddress(destination.to_canonical()),
                    },
                    ..demand
                };
                let RouteAction::Exit { nodes, via } =
                    matching::egress_action(&input, &normalized)?.0
                else {
                    return Err(CompileError::InvalidInput);
                };
                (
                    Target::Exit {
                        nodes: selectors(&nodes)?,
                        via: selectors(&via)?,
                    },
                    external_services(cfg, input.subjects, source, destination.to_canonical()),
                    Some(self.egress(source)?.capture.generation),
                )
            }
        };
        Ok(LocalDecision::Permitted(Decision {
            source,
            destination,
            target,
            services,
            paths: permit.paths,
            selection,
        }))
    }

    /// Every path of `origin` toward `target` that a relay may carry: the pair
    /// must have some Access service (Mesh) or some exit route with external
    /// Access (exit), exactly as when the endpoints decide.
    pub fn transit(
        &self,
        origin: NodeId,
        target: &Target,
    ) -> Result<Vec<LogicalPath>, CompileError> {
        let cfg = &self.configuration;
        let input = self.input()?;
        if !cfg.nodes.get(&origin).is_some_and(|n| n.enabled)
            || !input.inventory.contains_key(&origin)
        {
            return Ok(Vec::new());
        }
        let graph = self.graph(&input)?;
        match target {
            Target::Mesh { node } => {
                if !cfg.nodes.get(node).is_some_and(|n| n.enabled)
                    || crate::access::mesh_services(cfg, input.subjects, origin, *node).is_empty()
                {
                    return Ok(Vec::new());
                }
                search(&graph, origin, *node, None)
            }
            Target::Exit { nodes, via } => {
                if cfg.nodes[&origin].addresses.ipv4.is_none() {
                    return Ok(Vec::new());
                }
                let plan = self.egress(origin)?;
                let routes: Vec<ipnet::IpNet> = plan
                    .routes
                    .iter()
                    .filter(|route| {
                        matches!(&route.action, EgressAction::Exit { nodes: n, via: v } if n == nodes && v == via)
                    })
                    .filter_map(|route| route.prefix.parse().ok())
                    .filter(|net: &ipnet::IpNet| net.addr().is_ipv4())
                    .collect();
                if plan.capture.unavailable.is_some()
                    || !routes
                        .iter()
                        .any(|route| external_overlaps(cfg, input.subjects, origin, route))
                {
                    return Ok(Vec::new());
                }
                let mut paths = Vec::new();
                for exit in nodes {
                    if *exit == origin
                        || !cfg
                            .nodes
                            .get(exit)
                            .is_some_and(|n| n.enabled && n.exit_enabled)
                        || !exit::capable(input.topology(), *exit)
                    {
                        continue;
                    }
                    paths.extend(search(&graph, origin, *exit, Some(via))?);
                }
                Ok(paths)
            }
        }
    }

    /// Listener keys of `b` that `a` may dial under this projection.
    pub fn carriers(&self, a: NodeId, b: NodeId) -> Result<Vec<String>, CompileError> {
        let input = self.input()?;
        let graph = self.graph(&input)?;
        if !graph.topology_allows(a, b) {
            return Ok(Vec::new());
        }
        graph.listener_keys(a, b)
    }
}

fn external_scope(selector: &str) -> Option<IpScope> {
    if selector == "autogroup:internet" {
        Some(IpScope::Internet)
    } else {
        selector
            .parse::<ipnet::IpNet>()
            .ok()
            .filter(|net| net.addr().is_ipv4())
            .map(IpScope::Net)
    }
}

#[derive(Clone, Copy)]
enum IpScope {
    Internet,
    Net(ipnet::IpNet),
}

fn external_rules<'c>(
    cfg: &'c NetworkConfiguration,
    subjects: &'c SubjectContext,
    source: NodeId,
) -> impl Iterator<Item = (&'c AccessRule, IpScope)> {
    cfg.access
        .iter()
        .filter(move |rule| matching::selectors(cfg, subjects, &rule.src, source))
        .flat_map(|rule| {
            rule.dst
                .iter()
                .filter_map(move |s| external_scope(s).map(|scope| (rule, scope)))
        })
}

/// Union of every external service covering `destination` for `source`.
fn external_services(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    destination: IpAddr,
) -> ServiceSet {
    ServiceSet::from_grants(
        external_rules(cfg, subjects, source)
            .filter(|(_, scope)| match scope {
                IpScope::Internet => crate::is_internet_address(destination),
                IpScope::Net(net) => net.contains(&destination),
            })
            .flat_map(|(rule, _)| crate::access::token_services(&rule.ip)),
    )
}

fn external_overlaps(
    cfg: &NetworkConfiguration,
    subjects: &SubjectContext,
    source: NodeId,
    route: &ipnet::IpNet,
) -> bool {
    let overlap = |net: &ipnet::IpNet| net.contains(route) || route.contains(net);
    external_rules(cfg, subjects, source).any(|(rule, scope)| {
        !crate::access::token_services(&rule.ip)
            .collect::<Vec<_>>()
            .is_empty()
            && match scope {
                IpScope::Internet => crate::internet_ranges().iter().any(overlap),
                IpScope::Net(net) => overlap(&net),
            }
    })
}

/// Decide one new flow; see [`LocalCompiler::decide`].
pub fn compile_local(
    netmap: &NodeNetmap,
    source: NodeId,
    destination: IpAddr,
    service: (BusinessProtocol, Option<Port>),
) -> Result<LocalDecision, CompileError> {
    LocalCompiler::new(netmap).decide(source, destination, service.0, service.1)
}

/// Relay paths of a circuit; see [`LocalCompiler::transit`].
pub fn transit(
    netmap: &NodeNetmap,
    origin: NodeId,
    target: &Target,
) -> Result<Vec<LogicalPath>, CompileError> {
    LocalCompiler::new(netmap).transit(origin, target)
}
