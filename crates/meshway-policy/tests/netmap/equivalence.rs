//! Projection equivalence: for every (source, destination, protocol, port),
//! the per-demand compiler with its admission and the shared compiler run on
//! each Node's own projection reach the same decision, services, paths and
//! per-hop results; relays admit exactly the paths they appear on.
use super::*;
use meshway_policy::admission::netmap::{self as local, Leased};
use meshway_policy::admission::{self as old_admission, Traffic};
use std::net::IpAddr;

const DESTINATIONS: &[&str] = &[
    "8.8.8.8",
    "8.1.2.3",
    "10.20.1.1",
    "10.1.1.1",
    "1.1.1.1",
    "192.168.1.7",
    "127.0.0.1",
    "100.64.0.200",
    "2606:4700:4700::1111",
];
const SERVICES: &[(BusinessProtocol, Option<u16>)] = &[
    (BusinessProtocol::Tcp, Some(443)),
    (BusinessProtocol::Tcp, Some(8050)),
    (BusinessProtocol::Tcp, Some(22)),
    (BusinessProtocol::Udp, Some(53)),
    (BusinessProtocol::IcmpEcho, None),
    (BusinessProtocol::Tcp, Some(9999)),
];

#[derive(Default, Debug)]
pub(super) struct Coverage {
    pub scenarios: usize,
    pub node_projections: usize,
    pub queries: usize,
    pub mesh_permitted: usize,
    pub relayed: usize,
    pub transit_checks: usize,
    pub exit_permitted: usize,
    pub exit_backup: usize,
    pub direct: usize,
    pub unmanaged: usize,
    pub denied: usize,
    pub bidirectional_reverse: usize,
    pub disabled: usize,
    pub unregistered: usize,
    pub partial_projections: usize,
}

fn request(i: usize) -> RequestId {
    format!("00000000-0000-4001-8000-{i:012}").parse().unwrap()
}

struct Query {
    id: RequestId,
    source: NodeId,
    service: (BusinessProtocol, Option<Port>),
    traffic: Traffic,
}

fn old_compile(s: &Scenario, demands: &BTreeMap<RequestId, Demand>) -> CompiledPublication {
    compile_with_exit(
        CompilationInput {
            network: network(),
            revision: Revision::MIN,
            publication: "00000000-0000-4004-8000-000000000001".parse().unwrap(),
            configuration: &s.cfg,
            subjects: &s.subjects,
            inventory: &s.inventory,
            demands,
            dns: &[],
            rule_sets: &BTreeMap::new(),
            selected_profiles: &BTreeMap::new(),
            now_ms: 1000,
        },
        &BTreeMap::new(),
        &ExitInputs {
            instances: s
                .inventory
                .keys()
                .map(|n| (*n, n.to_string().parse().unwrap()))
                .collect(),
            instance_expires_at_ms: s.inventory.keys().map(|n| (*n, 100_000)).collect(),
            ready_until_ms: s.inventory.keys().map(|n| (*n, 100_000)).collect(),
            ..Default::default()
        },
    )
    .unwrap()
}

type PathShape = (Vec<NodeId>, BTreeMap<NodeId, Vec<String>>);

fn shape(paths: &[LogicalPath]) -> Vec<PathShape> {
    paths
        .iter()
        .map(|p| (p.nodes.clone(), p.dial_candidates.clone()))
        .collect()
}

fn same_hop(
    old: Result<old_admission::Hop, old_admission::AdmissionError>,
    new: Result<old_admission::Hop, old_admission::AdmissionError>,
) -> bool {
    match (old, new) {
        (Ok(a), Ok(b)) => a == b,
        (Err(_), Err(_)) => true,
        _ => false,
    }
}

struct World<'a> {
    old: &'a CompiledPublication,
    new: &'a CompiledNetmaps,
    leases: &'a BTreeMap<NodeId, NetmapLease>,
    compilers: &'a BTreeMap<NodeId, LocalCompiler>,
}

impl World<'_> {
    fn leased(&self, id: NodeId) -> Leased<'_> {
        Leased {
            netmap: &self.new.nodes[&id].netmap,
            lease: &self.leases[&id],
        }
    }

    /// Endpoint decision recomputed on `id`'s own projection.
    fn decision(&self, id: NodeId, query: &Query) -> LocalDecision {
        self.compilers[&id]
            .decide(
                query.source,
                query.traffic.destination,
                query.service.0,
                query.service.1,
            )
            .unwrap()
    }

    /// Per-hop admission of every allowed path, both directions, at every node.
    fn compare_hops(&self, query: &Query, source: &Decision, coverage: &mut Coverage) {
        let reply = Traffic {
            source: query.traffic.destination,
            destination: query.traffic.source,
            protocol: query.traffic.protocol,
            source_port: query.traffic.destination_port,
            destination_port: query.traffic.source_port,
        };
        for path in &source.paths {
            let nodes = &path.nodes;
            if nodes.len() > 2 {
                coverage.relayed += 1;
            }
            for (i, id) in nodes.iter().enumerate() {
                let cfg = &self.old.nodes[id].configuration;
                let leased = self.leased(*id);
                let previous = i.checked_sub(1).map(|p| nodes[p]);
                let next = nodes.get(i + 1).copied();
                let relay = i > 0 && i + 1 < nodes.len();
                let (old_forward, new_forward, old_reply, new_reply) = if relay {
                    coverage.transit_checks += 1;
                    let compiler = &self.compilers[id];
                    (
                        old_admission::admit_handshake(
                            cfg,
                            query.id,
                            nodes,
                            query.source,
                            previous,
                            false,
                            1001,
                        ),
                        local::admit_transit(
                            leased,
                            compiler,
                            query.source,
                            &source.target,
                            nodes,
                            previous,
                            false,
                            1001,
                        ),
                        old_admission::admit_handshake(
                            cfg,
                            query.id,
                            nodes,
                            query.source,
                            next,
                            true,
                            1001,
                        ),
                        local::admit_transit(
                            leased,
                            compiler,
                            query.source,
                            &source.target,
                            nodes,
                            next,
                            true,
                            1001,
                        ),
                    )
                } else {
                    let LocalDecision::Permitted(own) = self.decision(*id, query) else {
                        panic!("endpoint {id} does not permit {:?}", query.traffic);
                    };
                    assert_eq!(own, *source, "endpoint {id} decides differently");
                    (
                        old_admission::admit_forward(
                            cfg,
                            query.id,
                            nodes,
                            query.source,
                            previous,
                            &query.traffic,
                            1001,
                        ),
                        local::admit_forward(
                            leased,
                            &own,
                            nodes,
                            query.source,
                            previous,
                            &query.traffic,
                            1001,
                        ),
                        old_admission::admit_reply(
                            cfg,
                            query.id,
                            nodes,
                            query.source,
                            next,
                            &query.traffic,
                            &reply,
                            1001,
                        ),
                        local::admit_reply(
                            leased,
                            &own,
                            nodes,
                            query.source,
                            next,
                            &query.traffic,
                            &reply,
                            1001,
                        ),
                    )
                };
                assert!(
                    same_hop(old_forward, new_forward) && new_forward.is_ok(),
                    "forward at {i} of {nodes:?}: {old_forward:?} vs {new_forward:?}"
                );
                assert!(
                    same_hop(old_reply, new_reply),
                    "reply at {i} of {nodes:?}: {old_reply:?} vs {new_reply:?}"
                );
                let expired = NetmapLease::new(self.leases[id].netmap, 0, 1001).unwrap();
                let stale = Leased {
                    netmap: leased.netmap,
                    lease: &expired,
                };
                if relay {
                    assert!(
                        local::admit_transit(
                            stale,
                            &self.compilers[id],
                            query.source,
                            &source.target,
                            nodes,
                            previous,
                            false,
                            1001
                        )
                        .is_err()
                    );
                } else {
                    assert!(
                        local::admit_handshake(
                            stale,
                            source,
                            nodes,
                            query.source,
                            previous,
                            false,
                            1001
                        )
                        .is_err()
                    );
                }
            }
        }
    }

    /// Every relay of the source's paths computes exactly those paths; no
    /// other Node computes a path through itself.
    fn compare_transit(&self, source: &Decision) {
        let interior = |paths: &[LogicalPath], id: &NodeId| {
            paths
                .iter()
                .any(|p| p.nodes[1..p.nodes.len() - 1].contains(id))
        };
        for (id, compiler) in self.compilers {
            let paths = compiler.transit(source.source, &source.target).unwrap();
            if interior(&source.paths, id) {
                assert_eq!(shape(&paths), shape(&source.paths), "relay {id}");
            } else {
                assert!(
                    !interior(&paths, id),
                    "{id} would relay {:?}",
                    source.target
                );
            }
        }
    }
}

pub(super) fn check(seed: u64, coverage: &mut Coverage) {
    let s = generated(seed);
    let new = netmaps(&s);
    for compiled in new.nodes.values() {
        compiled.verify().unwrap();
    }
    let leases: BTreeMap<NodeId, NetmapLease> =
        new.nodes.iter().map(|(id, n)| (*id, lease(n))).collect();
    coverage.scenarios += 1;
    coverage.node_projections += new.nodes.len();
    coverage.partial_projections += new
        .nodes
        .values()
        .filter(|n| n.netmap.projection.configuration.nodes.len() < new.nodes.len())
        .count();
    coverage.disabled += s.cfg.nodes.values().filter(|n| !n.enabled).count();
    coverage.unregistered += s
        .cfg
        .nodes
        .keys()
        .filter(|id| !s.inventory.contains_key(id))
        .count();
    let mut destinations: Vec<IpAddr> = DESTINATIONS.iter().map(|d| d.parse().unwrap()).collect();
    destinations.extend(
        s.cfg
            .nodes
            .values()
            .filter_map(|n| n.addresses.ipv4.map(IpAddr::V4)),
    );
    let mut queries = Vec::new();
    let mut demands = BTreeMap::new();
    for source in new.nodes.keys() {
        let source_address = s.cfg.nodes[source].addresses.ipv4.unwrap();
        let profile = new.nodes[source].netmap.dns.profile.clone();
        for destination in &destinations {
            for (protocol, port_number) in SERVICES {
                let id = request(queries.len() + 1);
                let destination_port = port_number.and_then(port);
                demands.insert(
                    id,
                    Demand {
                        id,
                        source: *source,
                        target: BusinessTarget::Ip {
                            address: ExecutionAddress(*destination),
                        },
                        protocol: *protocol,
                        port: destination_port,
                        profile: profile.clone(),
                        expires_at_ms: 100_000,
                    },
                );
                queries.push(Query {
                    id,
                    source: *source,
                    service: (*protocol, destination_port),
                    traffic: Traffic {
                        source: IpAddr::V4(source_address),
                        destination: *destination,
                        protocol: *protocol,
                        source_port: port_number.and(port(40_000)),
                        destination_port,
                    },
                });
            }
        }
    }
    let ids: Vec<RequestId> = demands.keys().copied().collect();
    let mut olds = Vec::new();
    let mut decisions = BTreeMap::new();
    for chunk in ids.chunks(MAX_DEMANDS) {
        let batch: BTreeMap<_, _> = chunk.iter().map(|id| (*id, demands[id].clone())).collect();
        let old = old_compile(&s, &batch);
        for id in chunk {
            let source = demands[id].source;
            decisions.insert(
                *id,
                (
                    olds.len(),
                    old.nodes[&source].configuration.decisions[id].clone(),
                ),
            );
        }
        olds.push(old);
    }
    let permitted: BTreeMap<RequestId, bool> = decisions
        .iter()
        .map(|(id, (_, d))| (*id, matches!(d, DemandDecision::Permitted { .. })))
        .collect();
    let compilers: BTreeMap<NodeId, LocalCompiler> = new
        .nodes
        .iter()
        .map(|(id, n)| (*id, LocalCompiler::new(&n.netmap)))
        .collect();
    for query in &queries {
        coverage.queries += 1;
        let (batch, decision) = &decisions[&query.id];
        let world = World {
            old: &olds[*batch],
            new: &new,
            leases: &leases,
            compilers: &compilers,
        };
        let decided = world.decision(query.source, query);
        let context = || {
            format!(
                "seed {seed} {:?}: {decision:?} vs {decided:?}",
                query.traffic
            )
        };
        match (decision, &decided) {
            (DemandDecision::Permitted { permit }, LocalDecision::Permitted(own)) => {
                assert_eq!(shape(&permit.paths), shape(&own.paths), "{}", context());
                assert!(own.services.contains(query.service.0, query.service.1));
                // The service set equals the per-demand answers for this destination.
                for other in queries.iter().filter(|q| {
                    q.source == query.source && q.traffic.destination == query.traffic.destination
                }) {
                    assert_eq!(
                        own.services.contains(other.service.0, other.service.1),
                        permitted[&other.id],
                        "{}",
                        context()
                    );
                }
                match &own.target {
                    Target::Mesh { node } => {
                        coverage.mesh_permitted += 1;
                        if s.cfg.access.iter().any(|r| r.bidirectional)
                            && !crate_mesh_forward(&s, query.source, *node)
                        {
                            coverage.bidirectional_reverse += 1;
                        }
                    }
                    Target::Exit { nodes, .. } => {
                        coverage.exit_permitted += 1;
                        if nodes.len() > 1 {
                            coverage.exit_backup += 1;
                        }
                        assert!(own.selection.is_some());
                    }
                }
                world.compare_hops(query, own, coverage);
                world.compare_transit(own);
            }
            (DemandDecision::Direct, LocalDecision::Direct) => coverage.direct += 1,
            (DemandDecision::Unmanaged, LocalDecision::Unmanaged) => coverage.unmanaged += 1,
            (DemandDecision::Unavailable { .. }, LocalDecision::Denied(_)) => {
                coverage.denied += 1;
            }
            _ => panic!("decision mismatch: {}", context()),
        }
    }
    // A Mesh pair without any permitted service has no relay anywhere.
    for source in new.nodes.keys() {
        for (target, settings) in &s.cfg.nodes {
            let address = IpAddr::V4(settings.addresses.ipv4.unwrap());
            let any = queries.iter().any(|q| {
                q.source == *source && q.traffic.destination == address && permitted[&q.id]
            });
            if any || source == target {
                continue;
            }
            for compiler in compilers.values() {
                assert!(
                    compiler
                        .transit(*source, &Target::Mesh { node: *target })
                        .unwrap()
                        .is_empty(),
                    "seed {seed}: transit for unauthorized {source} -> {target}"
                );
            }
        }
    }
}

/// Whether a one-way rule alone grants the pair (else only a reverse rule does).
fn crate_mesh_forward(s: &Scenario, source: NodeId, target: NodeId) -> bool {
    s.cfg.access.iter().any(|r| {
        !r.bidirectional
            && r.src.iter().any(|x| s.subjects.matches(&s.cfg, x, source))
            && r.dst.iter().any(|x| s.subjects.matches(&s.cfg, x, target))
    })
}

#[test]
fn projection_compilation_equals_per_demand_compilation_for_generated_configurations() {
    let mut coverage = Coverage::default();
    for seed in 0..64 {
        check(seed, &mut coverage);
    }
    eprintln!("{coverage:#?}");
    assert_eq!(coverage.scenarios, 64);
    for (name, value) in [
        ("mesh", coverage.mesh_permitted),
        ("relayed", coverage.relayed),
        ("transit", coverage.transit_checks),
        ("exit", coverage.exit_permitted),
        ("exit backup", coverage.exit_backup),
        ("direct", coverage.direct),
        ("unmanaged", coverage.unmanaged),
        ("denied", coverage.denied),
        ("bidirectional", coverage.bidirectional_reverse),
        ("disabled", coverage.disabled),
        ("unregistered", coverage.unregistered),
        ("partial projection", coverage.partial_projections),
    ] {
        assert!(value > 0, "generated configurations never covered {name}");
    }
}
