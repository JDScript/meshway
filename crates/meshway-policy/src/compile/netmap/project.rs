//! Server-side projection of one configuration for one Node.
//!
//! Visible Nodes V(n): the topology component of n among available Nodes
//! (every path n can terminate or relay lies in it), Nodes with an Access rule
//! toward or from n, and via waypoints of the routing rules kept below. A rule
//! is kept iff it can match a pair inside V; dropped rules cannot match any
//! segment, edge or flow the Node evaluates, so first-match order is unchanged.
use super::*;
use ipnet::IpNet;

/// Per-rule selector expansions and components, computed once per compilation.
pub(super) struct Index {
    component: BTreeMap<NodeId, usize>,
    components: Vec<Vec<NodeId>>,
    access: Vec<(BTreeSet<NodeId>, BTreeSet<NodeId>, bool)>,
    topology: Vec<(BTreeSet<NodeId>, BTreeSet<NodeId>)>,
    mesh: Vec<(BTreeSet<NodeId>, BTreeSet<NodeId>, Vec<NodeId>)>,
    views: BTreeMap<NodeId, NodeSettingsView>,
}

fn external(selector: &str) -> bool {
    selector == "autogroup:internet"
        || selector.starts_with("domain:")
        || selector.parse::<IpNet>().is_ok()
}

impl Index {
    pub fn new(
        input: &NetmapInput<'_>,
        graph: &paths::Graph,
        registered: &[NodeId],
    ) -> Result<Self, CompileError> {
        let cfg = input.configuration;
        let matching = |selectors: &[String]| -> BTreeSet<NodeId> {
            registered
                .iter()
                .copied()
                .filter(|id| matching::selectors(cfg, input.subjects, selectors, *id))
                .collect()
        };
        let available: Vec<NodeId> = registered
            .iter()
            .copied()
            .filter(|id| cfg.nodes[id].enabled)
            .collect();
        // Union-find over topology-allowed edges; carriers are a subset.
        let mut parent: Vec<usize> = (0..available.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for i in 0..available.len() {
            for j in i + 1..available.len() {
                if root(&mut parent, i) != root(&mut parent, j)
                    && graph.topology_allows(available[i], available[j])
                {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
        let mut ids = BTreeMap::new();
        let mut components: Vec<Vec<NodeId>> = Vec::new();
        let mut component = BTreeMap::new();
        for (i, node) in available.iter().enumerate() {
            let r = root(&mut parent, i);
            let index = *ids.entry(r).or_insert_with(|| {
                components.push(Vec::new());
                components.len() - 1
            });
            components[index].push(*node);
            component.insert(*node, index);
        }
        let nodes = |selectors: &[String]| {
            selectors
                .iter()
                .filter_map(|s| matching::node_selector(s).ok())
                .collect::<Vec<_>>()
        };
        Ok(Self {
            component,
            components,
            access: cfg
                .access
                .iter()
                .map(|r| {
                    (
                        matching(&r.src),
                        matching(&r.dst),
                        r.dst.iter().any(|s| external(s)),
                    )
                })
                .collect(),
            topology: cfg
                .topology
                .iter()
                .map(|r| {
                    (
                        matching(std::slice::from_ref(&r.a)),
                        matching(std::slice::from_ref(&r.b)),
                    )
                })
                .collect(),
            mesh: cfg
                .routing
                .mesh
                .iter()
                .map(|r| (matching(&r.src), matching(&r.dst), nodes(&r.via)))
                .collect(),
            views: registered
                .iter()
                .map(|id| (*id, NodeSettingsView::from(&cfg.nodes[id])))
                .collect(),
        })
    }

    /// Visible Nodes of `node`.
    pub fn visible(&self, node: NodeId) -> BTreeSet<NodeId> {
        let mut visible = BTreeSet::from([node]);
        if let Some(index) = self.component.get(&node) {
            visible.extend(&self.components[*index]);
        }
        for (src, dst, _) in &self.access {
            if src.contains(&node) {
                visible.extend(dst);
            }
            if dst.contains(&node) {
                visible.extend(src);
            }
        }
        // Waypoints of kept routing rules; bounded by the rule count.
        loop {
            let before = visible.len();
            for (src, dst, via) in &self.mesh {
                if meets(src, &visible) && meets(dst, &visible) {
                    visible.extend(via.iter().filter(|id| self.views.contains_key(id)));
                }
            }
            if visible.len() == before {
                break;
            }
        }
        visible.retain(|id| self.views.contains_key(id));
        visible
    }
}

fn meets(set: &BTreeSet<NodeId>, visible: &BTreeSet<NodeId>) -> bool {
    if set.len() < visible.len() {
        set.iter().any(|id| visible.contains(id))
    } else {
        visible.iter().any(|id| set.contains(id))
    }
}

/// The projection of `input` for `node`; a monotone subset of the full input.
pub fn project(input: &NetmapInput<'_>, index: &Index, node: NodeId) -> ProjectedConfiguration {
    let cfg = input.configuration;
    let visible = index.visible(node);
    let keep = |rules: usize, predicate: &dyn Fn(usize) -> bool| -> Vec<usize> {
        (0..rules).filter(|i| predicate(*i)).collect()
    };
    let access = keep(cfg.access.len(), &|i| {
        let (src, dst, external) = &index.access[i];
        meets(src, &visible) && (*external || meets(dst, &visible))
    });
    let topology = keep(cfg.topology.len(), &|i| {
        let (a, b) = &index.topology[i];
        meets(a, &visible) && meets(b, &visible)
    });
    let mesh = keep(cfg.routing.mesh.len(), &|i| {
        let (src, dst, _) = &index.mesh[i];
        meets(src, &visible) && meets(dst, &visible)
    });
    let bindings: Vec<&UserBinding> = visible
        .iter()
        .filter_map(|id| input.subjects.node_users.get(id))
        .collect();
    let users: BTreeSet<&String> = visible
        .iter()
        .filter_map(|id| match &cfg.nodes[id].identity {
            NodeIdentity::User { user } => Some(user),
            NodeIdentity::Tag { .. } => None,
        })
        .collect();
    let subjects = SubjectContext {
        users: input
            .subjects
            .users
            .iter()
            .filter(|(_, binding)| bindings.contains(binding))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        node_users: input
            .subjects
            .node_users
            .iter()
            .filter(|(id, _)| visible.contains(id))
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        members: input
            .subjects
            .members
            .iter()
            .filter(|(user, _)| users.contains(user))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    let pick = |map: &BTreeMap<NodeId, _>| -> Vec<NodeId> {
        visible
            .iter()
            .copied()
            .filter(|id| map.contains_key(id))
            .collect()
    };
    ProjectedConfiguration {
        configuration: ConfigurationView {
            address_pools: cfg.address_pools.clone(),
            defaults: (&cfg.defaults).into(),
            groups: cfg.groups.clone(),
            tags_owners: cfg.tags_owners.clone(),
            access: access.iter().map(|i| cfg.access[*i].clone()).collect(),
            nodes: visible
                .iter()
                .map(|id| (*id, index.views[id].clone()))
                .collect(),
            dns: cfg.dns.clone(),
            topology: topology.iter().map(|i| cfg.topology[*i].clone()).collect(),
            routing: RoutingConfiguration {
                mesh: mesh.iter().map(|i| cfg.routing.mesh[*i].clone()).collect(),
                egress: cfg.routing.egress.clone(),
            },
            rule_sets: cfg.rule_sets.clone(),
        },
        subjects,
        inventory: pick(input.inventory)
            .into_iter()
            .map(|id| (id, input.inventory[&id].clone()))
            .collect(),
        rule_sets: input.rule_sets.clone(),
        selected_profiles: visible
            .iter()
            .filter_map(|id| input.selected_profiles.get(id).map(|p| (*id, p.clone())))
            .collect(),
        selection_versions: visible
            .iter()
            .filter_map(|id| input.selection_versions.get(id).map(|v| (*id, *v)))
            .collect(),
    }
}
