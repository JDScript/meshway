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
        let position: BTreeMap<NodeId, usize> = available
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        let join = |parent: &mut Vec<usize>, i: usize, j: usize| {
            let (a, b) = (root(parent, i), root(parent, j));
            if a != b {
                parent[a] = b;
            }
        };
        if cfg
            .topology
            .iter()
            .any(|rule| rule.action == TopologyAction::Block)
        {
            // A block rule removes single edges: decide every pair.
            for i in 0..available.len() {
                for j in i + 1..available.len() {
                    if root(&mut parent, i) != root(&mut parent, j)
                        && graph.topology_allows(available[i], available[j])
                    {
                        join(&mut parent, i, j);
                    }
                }
            }
        } else if cfg.defaults.topology == TopologyDefault::Allow {
            for i in 1..available.len() {
                join(&mut parent, 0, i);
            }
        } else {
            // Allow rules only: rule r joins every a in A_r with every b in
            // B_r (a != b), so A_r and B_r form one component whenever such
            // a pair exists. Linear in rules and Nodes instead of all pairs.
            for rule in &cfg.topology {
                let members: BTreeSet<usize> = [&rule.a, &rule.b]
                    .into_iter()
                    .flat_map(|selector| {
                        available
                            .iter()
                            .filter(|id| {
                                matching::selectors(
                                    cfg,
                                    input.subjects,
                                    std::slice::from_ref(selector),
                                    **id,
                                )
                            })
                            .map(|id| position[id])
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let side = |selector: &String| {
                    available
                        .iter()
                        .filter(|id| {
                            matching::selectors(
                                cfg,
                                input.subjects,
                                std::slice::from_ref(selector),
                                **id,
                            )
                        })
                        .count()
                };
                if members.len() >= 2 && side(&rule.a) > 0 && side(&rule.b) > 0 {
                    let mut members = members.into_iter();
                    let first = members.next().expect("two members");
                    for other in members {
                        join(&mut parent, first, other);
                    }
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
    // Disclosure: only what a visible Node's evaluation can reach. Profile
    // assignments that match no visible Node cannot be the first match of one;
    // profiles outside kept assignments, selections and `auto` are never
    // evaluated. Groups and tags keep the names kept rules reference (their
    // validation needs them) and members/owners among the visible users.
    let egress = &cfg.routing.egress;
    let assignments: Vec<ProfileAssignment> = egress
        .assignments
        .iter()
        .filter(|assignment| {
            visible
                .iter()
                .any(|id| matching::selectors(cfg, input.subjects, &assignment.target, *id))
        })
        .cloned()
        .collect();
    let mut profiles_used: BTreeSet<&str> = assignments
        .iter()
        .flat_map(|assignment| {
            assignment
                .allowed
                .iter()
                .chain(std::iter::once(&assignment.default))
        })
        .map(String::as_str)
        .collect();
    profiles_used.extend(
        visible
            .iter()
            .filter_map(|id| input.selected_profiles.get(id))
            .map(String::as_str),
    );
    if let Input::Value(auto) = &egress.auto {
        profiles_used.extend(auto.rules.iter().map(|rule| rule.profile.as_str()));
        profiles_used.insert(auto.fallback.as_str());
    }
    // Rule sets: only those a kept public or profile rule matches on.
    let kept_profiles: Vec<&RoutingProfile> = egress
        .profiles
        .iter()
        .filter(|(name, _)| profiles_used.contains(name.as_str()))
        .map(|(_, profile)| profile)
        .collect();
    let used_sets: BTreeSet<&str> = egress
        .rules
        .iter()
        .chain(kept_profiles.iter().flat_map(|profile| &profile.rules))
        .flat_map(|rule| &rule.matcher.rule_sets)
        .map(String::as_str)
        .collect();
    let selectors: Vec<&String> = access
        .iter()
        .flat_map(|i| cfg.access[*i].src.iter().chain(&cfg.access[*i].dst))
        .chain(
            topology
                .iter()
                .flat_map(|i| [&cfg.topology[*i].a, &cfg.topology[*i].b]),
        )
        .chain(mesh.iter().flat_map(|i| {
            let rule = &cfg.routing.mesh[*i];
            rule.src.iter().chain(&rule.dst).chain(&rule.via)
        }))
        .chain(assignments.iter().flat_map(|assignment| &assignment.target))
        .collect();
    let referenced = |prefix: &str| -> BTreeSet<&str> {
        selectors
            .iter()
            .filter_map(|selector| selector.strip_prefix(prefix))
            .collect()
    };
    let (group_names, tag_names) = (referenced("group:"), referenced("tag:"));
    let visible_tags: BTreeSet<&String> = visible
        .iter()
        .filter_map(|id| match &cfg.nodes[id].identity {
            NodeIdentity::Tag { tags } => Some(tags),
            NodeIdentity::User { .. } => None,
        })
        .flatten()
        .collect();
    let groups = cfg
        .groups
        .iter()
        .filter(|(name, members)| {
            name.strip_prefix("group:")
                .is_some_and(|name| group_names.contains(name))
                || members.iter().any(|m| users.contains(m))
        })
        .map(|(name, members)| {
            (
                name.clone(),
                members
                    .iter()
                    .filter(|m| users.contains(m))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let tags_owners = cfg
        .tags_owners
        .iter()
        .filter(|(tag, _)| {
            visible_tags.contains(tag)
                || tag
                    .strip_prefix("tag:")
                    .is_some_and(|name| tag_names.contains(name))
        })
        .map(|(tag, owners)| {
            (
                tag.clone(),
                owners
                    .iter()
                    .filter(|o| users.contains(o))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let pick = |map: &BTreeMap<NodeId, _>| -> Vec<NodeId> {
        visible
            .iter()
            .copied()
            .filter(|id| map.contains_key(id))
            .collect()
    };
    let routing = RoutingConfiguration {
        mesh: mesh.iter().map(|i| cfg.routing.mesh[*i].clone()).collect(),
        egress: EgressRouting {
            rules: egress.rules.clone(),
            profiles: egress
                .profiles
                .iter()
                .filter(|(name, _)| profiles_used.contains(name.as_str()))
                .map(|(name, profile)| (name.clone(), profile.clone()))
                .collect(),
            assignments,
            auto: egress.auto.clone(),
        },
    };
    ProjectedConfiguration {
        eligible_exits: matching::eligible_exits(cfg, &routing.egress),
        configuration: ConfigurationView {
            address_pools: cfg.address_pools.clone(),
            defaults: (&cfg.defaults).into(),
            groups,
            tags_owners,
            access: access.iter().map(|i| cfg.access[*i].clone()).collect(),
            nodes: visible
                .iter()
                .map(|id| (*id, index.views[id].clone()))
                .collect(),
            dns: cfg.dns.clone(),
            topology: topology.iter().map(|i| cfg.topology[*i].clone()).collect(),
            routing,
            // Nodes judge sets by content, fetched by digest: definitions
            // (source URLs, catalogs, inline lists) stay on the Server.
            rule_sets: BTreeMap::new(),
            rule_set_providers: BTreeMap::new(),
        },
        subjects,
        inventory: pick(input.inventory)
            .into_iter()
            .map(|id| (id, input.inventory[&id].clone()))
            .collect(),
        rule_sets: input
            .rule_sets
            .iter()
            .filter(|(key, _)| used_sets.contains(key.as_str()))
            .map(|(key, set)| (key.clone(), super::super::rules::reference(set)))
            .collect(),
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
