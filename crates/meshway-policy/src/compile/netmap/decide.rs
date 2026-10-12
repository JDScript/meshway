//! Node-side compilation over an installed netmap's projection: the same
//! routing and Access evaluation, without dynamic exit facts or server grants.
use super::*;
use crate::compile::{routing::PathError, rules};
use meshway_transport::spec::Port;
use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
};

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
    pub(super) netmap: Arc<NodeNetmap>,
    pub(super) configuration: NetworkConfiguration,
    /// Referenced sets whose artifact this Node holds; a missing one is an
    /// unavailable dependency of every rule naming it.
    pub(super) rule_sets: rules::RuleSets,
    pub(super) matchers: rules::Matchers,
    /// Partition of each profile computed so far: a pure function of this
    /// content and the loaded sets, bounded by the projection's profiles. A
    /// source its profile names as an exit executes its own partition, keyed
    /// with that source.
    partitions: Mutex<BTreeMap<(String, Option<NodeId>), crate::compile::Partition>>,
}

/// The egress rule that decides a destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressRuleHit {
    /// JSON Pointer of the rule in the configuration.
    pub rule: String,
    /// The rule-set member through which it matched, when it did.
    pub rule_set: Option<meshway_configuration::evaluation::RuleSetMatch>,
    pub action: RouteAction,
}

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
        rule_set_providers: view.rule_set_providers.clone(),
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

impl LocalCompiler {
    pub fn new(netmap: &Arc<NodeNetmap>, artifacts: &rules::Artifacts) -> Self {
        Self::from_shared(netmap.clone(), artifacts)
    }

    /// Own immutable content so a runtime may retain one compiler across
    /// requests. A compiler computes decisions; it never carries authority.
    /// `artifacts` are the loaded rule sets; the projection's references to
    /// others are unavailable dependencies.
    pub fn from_shared(netmap: Arc<NodeNetmap>, artifacts: &rules::Artifacts) -> Self {
        let configuration = rebuild(&netmap.projection.configuration);
        // Validated on the Server; a list that does not convert matches nothing
        // and every rule depending on it is unavailable.
        let matchers =
            rules::Matchers::new(&configuration).unwrap_or_else(|_| rules::Matchers::unavailable());
        Self {
            rule_sets: rules::resolve(&netmap.projection.rule_sets, artifacts),
            matchers,
            configuration,
            netmap,
            partitions: Mutex::new(BTreeMap::new()),
        }
    }

    /// This Node's netmap as it executes with the artifacts it holds. With
    /// every referenced artifact loaded it is the signed content itself.
    /// Otherwise its own egress plan is recomputed here, which fails closed
    /// for a rule naming a missing set (full IPv4 capture, IPv6 sink, no
    /// direct or unmanaged exception); resolver sockets follow that plan and
    /// a Mesh DNS service bound to the signed plan is unavailable.
    pub fn effective(&self) -> Result<Arc<NodeNetmap>, CompileError> {
        if self.missing().is_empty() {
            return Ok(Arc::clone(&self.netmap));
        }
        let mut netmap = (*self.netmap).clone();
        if netmap.egress.is_some() {
            let plan = self.egress(netmap.node_id)?;
            netmap.dns.resolvers =
                local::resolvers(&self.configuration, true, &self.partition(netmap.node_id)?);
            if netmap.egress.as_ref() != Some(&plan)
                && matches!(netmap.dns.egress, DnsPlan::ViaExit { .. })
            {
                netmap.dns.egress = DnsPlan::Unavailable {
                    reason:
                        meshway_configuration::execution::DnsEgressUnavailable::ExitPlanRequired,
                };
            }
            netmap.egress = Some(plan);
        }
        Ok(Arc::new(netmap))
    }

    /// The partition of `source`'s effective profile with the sets this Node
    /// holds; `Err` is why none exists (capture is then the full range).
    pub fn partition(&self, source: NodeId) -> Result<crate::compile::Partition, CompileError> {
        let projection = &self.netmap.projection;
        let profile = effective_profile(
            &self.configuration,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        let named = crate::compile::capture::names_exit(&self.configuration, &profile, source)
            .then_some(source);
        let key = (profile, named);
        let mut partitions = self
            .partitions
            .lock()
            .map_err(|_| CompileError::InvalidInput)?;
        if let Some(partition) = partitions.get(&key) {
            return Ok(partition.clone());
        }
        let partition =
            crate::compile::capture::partition(&self.configuration, &self.rule_sets, &key.0, named)
                .map(Arc::new);
        partitions.insert(key, partition.clone());
        Ok(partition)
    }

    /// What `source` installs for its egress capture: the partition's counts,
    /// over the limits included, or the fail-closed boundary's with its reason.
    /// The Server previews the same numbers (`ProfileEvaluation.capture`).
    pub fn capture_summary(
        &self,
        source: NodeId,
    ) -> Result<meshway_configuration::evaluation::CaptureSummary, CompileError> {
        let projection = &self.netmap.projection;
        let profile = effective_profile(
            &self.configuration,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        Ok(crate::compile::capture_summary(
            &self.configuration,
            &self.rule_sets,
            &profile,
            Some(source),
        ))
    }

    /// The capture boundary this Node installs for its own egress plan.
    pub fn capture(&self) -> Result<Option<EgressCapturePlan>, CompileError> {
        let node = self.netmap.node_id;
        if self.netmap.egress.is_none() {
            return Ok(None);
        }
        let generation = self.egress(node)?.generation;
        Ok(Some(match self.partition(node)? {
            Ok(partition) => partition.capture(generation),
            Err(reason) => crate::compile::unavailable_capture(generation, reason),
        }))
    }

    /// Referenced artifacts this compiler does not hold.
    pub fn missing(&self) -> Vec<meshway_rules::Digest> {
        let mut missing: Vec<_> = self
            .netmap
            .projection
            .rule_sets
            .iter()
            .filter(|(key, _)| !self.rule_sets.contains_key(*key))
            .map(|(_, reference)| reference.digest)
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    /// The first egress rule of `source` selecting `destination`; `None` when
    /// the profile fallback applies. A referenced set this Node does not hold
    /// is `UnavailableDependency`: evaluation never skips past it.
    pub fn egress_rule_for_ip(
        &self,
        source: NodeId,
        destination: IpAddr,
    ) -> Result<Option<EgressRuleHit>, CompileError> {
        self.egress_rule(
            source,
            rules::Destination::Address(destination.to_canonical()),
        )
    }

    /// Same for a DNS query name, compared in [`rules::query_name`] form.
    pub fn egress_rule_for_name(
        &self,
        source: NodeId,
        name: &str,
    ) -> Result<Option<EgressRuleHit>, CompileError> {
        let name = rules::query_name(name);
        self.egress_rule(source, rules::Destination::Name(&name))
    }

    fn egress_rule(
        &self,
        source: NodeId,
        destination: rules::Destination<'_>,
    ) -> Result<Option<EgressRuleHit>, CompileError> {
        let cfg = &self.configuration;
        let projection = &self.netmap.projection;
        let mut profile = effective_profile(
            cfg,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        let mut skipped = Vec::new();
        let public = "/routing/egress/rules";
        let hit = |index: usize,
                   rule_set,
                   rules: &[EgressRule],
                   prefix: &str|
         -> Result<EgressRuleHit, CompileError> {
            Ok(EgressRuleHit {
                rule: format!("{prefix}/{index}"),
                rule_set,
                action: matching::executed(rules[index].action.clone().into(), source)?,
            })
        };
        let rules_list = &cfg.routing.egress.rules;
        if let Some((index, set)) = rules::first_match(
            rules_list,
            self.matchers.of(None)?,
            &self.rule_sets,
            destination,
            public,
            &mut skipped,
        )? {
            return hit(index, set, rules_list, public).map(Some);
        }
        if let matching::EgressProfile::Auto(auto) = matching::resolve_profile(cfg, &profile)? {
            profile = auto.fallback.clone();
        }
        let matching::EgressProfile::Custom(custom) = matching::resolve_profile(cfg, &profile)?
        else {
            return Ok(None);
        };
        let prefix = format!(
            "/routing/egress/profiles/{}/rules",
            profile.replace('~', "~0").replace('/', "~1")
        );
        rules::first_match(
            &custom.rules,
            self.matchers.of(Some(&profile))?,
            &self.rule_sets,
            destination,
            &prefix,
            &mut skipped,
        )?
        .map(|(index, set)| hit(index, set, &custom.rules, &prefix))
        .transpose()
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
            rule_sets: &self.rule_sets,
            selected_profiles: &projection.selected_profiles,
            selection_versions: &projection.selection_versions,
            previous: &BTreeMap::new(),
        };
        let source_local = local::Local::compile(&input, source)?;
        let Some((exit, _)) =
            super::super::dns::single_exit(&self.configuration, &source_local.dns.profile, source)?
        else {
            return Ok(None);
        };
        if !self.configuration.nodes.contains_key(&exit)
            || !projection.inventory.contains_key(&exit)
        {
            return Ok(None);
        }
        let graph = paths::Graph::projected(input.topology(), &projection.configuration)?;
        Ok(local::service(&input, &graph, &source_local, source)?.ok())
    }

    fn topology(&self) -> Topology<'_> {
        let projection = &self.netmap.projection;
        Topology {
            configuration: &self.configuration,
            subjects: &projection.subjects,
            inventory: &projection.inventory,
        }
    }

    fn graph(&self) -> Result<paths::Graph<'_>, CompileError> {
        paths::Graph::projected(self.topology(), &self.netmap.projection.configuration)
    }

    /// Egress plan of a visible source, identical to the one its own netmap
    /// carries when this Node holds every artifact the source's rules use.
    pub fn egress(&self, source: NodeId) -> Result<EgressPlan, CompileError> {
        let projection = &self.netmap.projection;
        let profile = effective_profile(
            &self.configuration,
            &projection.subjects,
            &projection.selected_profiles,
            source,
        )?;
        let references = projection
            .rule_sets
            .iter()
            .map(|(key, reference)| (key.clone(), reference.digest))
            .collect();
        local::egress(
            self.netmap.network_id,
            &self.configuration,
            &references,
            &self.partition(source)?,
            source,
            &profile,
            projection
                .selection_versions
                .get(&source)
                .map_or(0, |version| version.get()),
        )
    }

    /// Decide one new flow of `source` toward `destination` without DNS context.
    pub fn decide(
        &self,
        source: NodeId,
        destination: IpAddr,
        protocol: BusinessProtocol,
        port: Option<Port>,
    ) -> Result<LocalDecision, CompileError> {
        self.decide_with(
            source,
            destination,
            protocol,
            port,
            &Associations::default(),
        )
    }

    /// Decide one new flow of `source` toward `destination`. An address the
    /// source's trusted associations name is decided by first match over the
    /// address and those names (all names must agree, else `egressConflict`),
    /// with `domain:` grants of the names; any other address by the IP
    /// partition alone.
    pub fn decide_with(
        &self,
        source: NodeId,
        destination: IpAddr,
        protocol: BusinessProtocol,
        port: Option<Port>,
        associations: &Associations,
    ) -> Result<LocalDecision, CompileError> {
        if (protocol == BusinessProtocol::IcmpEcho) != port.is_none() {
            return Ok(LocalDecision::Denied(UnavailableReason::Unauthorized));
        }
        let cfg = &self.configuration;
        let input = self.topology();
        if !cfg.nodes.get(&source).is_some_and(|node| node.enabled)
            || !input.inventory.contains_key(&source)
        {
            return Ok(LocalDecision::Denied(UnavailableReason::InactiveNode));
        }
        let canonical = destination.to_canonical();
        let mesh = cfg
            .nodes
            .iter()
            .find(|(_, node)| {
                node.addresses.ipv4.map(IpAddr::V4) == Some(canonical)
                    || node.addresses.ipv6.map(IpAddr::V6) == Some(canonical)
            })
            .map(|(id, _)| *id);
        let (target, services, selection) = if let Some(node) = mesh {
            (
                Target::Mesh { node },
                crate::access::mesh_services(cfg, input.subjects, source, node),
                None,
            )
        } else {
            if cfg
                .address_pools
                .ipv4
                .iter()
                .chain(&cfg.address_pools.ipv6)
                .any(|pool| {
                    pool.parse::<ipnet::IpNet>()
                        .is_ok_and(|pool| pool.contains(&canonical))
                })
            {
                return Ok(LocalDecision::Denied(UnavailableReason::NoPath));
            }
            let partition = match self.partition(source)? {
                Ok(partition) => partition,
                Err(reason) => return Ok(LocalDecision::Denied(reason)),
            };
            let generation = self.egress(source)?.generation;
            let names = associations.names(source, generation, canonical);
            let associated;
            let action = if names.is_empty() {
                partition.action(canonical)
            } else {
                associated = match self.associated_action(source, canonical, &names)? {
                    Some(action) => crate::compile::capture::action(&action)?,
                    None => return Ok(LocalDecision::Denied(UnavailableReason::EgressConflict)),
                };
                &associated
            };
            let (nodes, via) = match action {
                EgressAction::Direct => return Ok(LocalDecision::Direct),
                EgressAction::Unmanaged => return Ok(LocalDecision::Unmanaged),
                EgressAction::Exit { nodes, via } => (nodes.clone(), via.clone()),
            };
            let services = if names.is_empty() {
                external_services(cfg, input.subjects, source, canonical)
            } else {
                self.associated_services(source, canonical, &names)
            };
            if !services.contains(protocol, port) {
                return Ok(LocalDecision::Denied(UnavailableReason::Unauthorized));
            }
            if destination.is_ipv6() || cfg.nodes[&source].addresses.ipv4.is_none() {
                return Ok(LocalDecision::Denied(UnavailableReason::UnsupportedFamily));
            }
            if !crate::is_exit_destination(canonical) {
                return Ok(LocalDecision::Denied(UnavailableReason::Unauthorized));
            }
            (Target::Exit { nodes, via }, services, Some(generation))
        };
        if !services.contains(protocol, port) {
            return Ok(LocalDecision::Denied(UnavailableReason::Unauthorized));
        }
        let (targets, via) = match &target {
            Target::Mesh { node } => (std::slice::from_ref(node), None),
            Target::Exit { nodes, via } => (nodes.as_slice(), Some(via.as_slice())),
        };
        if targets.len() > MAX_EXIT_CANDIDATES {
            return Err(CompileError::Budget);
        }
        let graph = self.graph()?;
        let mut paths = Vec::new();
        let mut reason = UnavailableReason::NoPath;
        for target in targets {
            if !cfg
                .nodes
                .get(target)
                .is_some_and(|node| node.enabled && (via.is_none() || node.exit_enabled))
                || (via.is_some() && *target == source)
            {
                continue;
            }
            if via.is_some() && !exit::capable(input, *target) {
                reason = UnavailableReason::ExitUnavailable;
                continue;
            }
            match graph.paths(source, *target, via, MAX_PATHS) {
                Ok(found) => paths.extend(found),
                Err(PathError::Cycle) => reason = UnavailableReason::RoutingCycle,
                Err(PathError::Limit) => reason = UnavailableReason::RoutingLimit,
                Err(PathError::Compile(CompileError::EdgeTransportUnavailable { .. })) => {}
                Err(PathError::Compile(error)) => return Err(error),
            }
        }
        if paths.is_empty() {
            return Ok(LocalDecision::Denied(reason));
        }
        Ok(LocalDecision::Permitted(Decision {
            source,
            destination,
            target,
            services,
            paths,
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
        let input = self.topology();
        if !cfg.nodes.get(&origin).is_some_and(|n| n.enabled)
            || !input.inventory.contains_key(&origin)
        {
            return Ok(Vec::new());
        }
        let graph = self.graph()?;
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
                let Ok(partition) = self.partition(origin)? else {
                    return Ok(Vec::new());
                };
                if !partition
                    .ipv4()
                    .filter(|(_, action)| {
                        matches!(action, EgressAction::Exit { nodes: n, via: v } if n == nodes && v == via)
                    })
                    .any(|(range, _)| external_overlaps(cfg, input.subjects, origin, &range))
                    && !self.name_exit(origin, nodes, via)?
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
                        || !exit::capable(input, *exit)
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
        let graph = self.graph()?;
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
    range: &std::ops::RangeInclusive<std::net::Ipv4Addr>,
) -> bool {
    let overlap = |net: &ipnet::IpNet| match net {
        ipnet::IpNet::V4(net) => net.network() <= *range.end() && *range.start() <= net.broadcast(),
        ipnet::IpNet::V6(_) => false,
    };
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
    netmap: &Arc<NodeNetmap>,
    artifacts: &rules::Artifacts,
    source: NodeId,
    destination: IpAddr,
    service: (BusinessProtocol, Option<Port>),
) -> Result<LocalDecision, CompileError> {
    LocalCompiler::new(netmap, artifacts).decide(source, destination, service.0, service.1)
}

/// Relay paths of a circuit; see [`LocalCompiler::transit`].
pub fn transit(
    netmap: &Arc<NodeNetmap>,
    artifacts: &rules::Artifacts,
    origin: NodeId,
    target: &Target,
) -> Result<Vec<LogicalPath>, CompileError> {
    LocalCompiler::new(netmap, artifacts).transit(origin, target)
}
