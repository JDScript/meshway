//! Differential equivalence: for every (source, destination, protocol, port)
//! the per-demand compiler with its admission and the netmap compiler with
//! local admission reach the same decision, paths and per-hop results.
use super::*;
use meshway_policy::admission::netmap::{self as local, Destination, Leased};
use meshway_policy::admission::{self as old_admission, Traffic};
use std::collections::BTreeSet;
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
    pub queries: usize,
    pub mesh_permitted: usize,
    pub relayed: usize,
    pub exit_permitted: usize,
    pub exit_backup: usize,
    pub direct: usize,
    pub unmanaged: usize,
    pub denied: usize,
    pub bidirectional_reverse: usize,
    pub disabled: usize,
    pub unregistered: usize,
}

fn request(i: usize) -> RequestId {
    format!("00000000-0000-4001-8000-{i:012}").parse().unwrap()
}

struct Query {
    id: RequestId,
    source: NodeId,
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

fn shape(paths: impl IntoIterator<Item = PathShape>) -> Vec<PathShape> {
    paths.into_iter().collect()
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

/// Per-hop admission of every allowed path, both directions, at every node.
fn compare_hops(
    old: &CompiledPublication,
    new: &CompiledNetmaps,
    leases: &BTreeMap<NodeId, NetmapLease>,
    query: &Query,
    keys: &[GrantKey],
    permit: &PathPermit,
    coverage: &mut Coverage,
) {
    for path in &permit.paths {
        let nodes = &path.nodes;
        let terminal = *nodes.last().unwrap();
        let key = keys
            .iter()
            .find(|key| match key.target {
                GrantTarget::Mesh { node } | GrantTarget::Exit { node, .. } => node == terminal,
            })
            .expect("grant for path terminal");
        if nodes.len() > 2 {
            coverage.relayed += 1;
        }
        let reply = Traffic {
            source: query.traffic.destination,
            destination: query.traffic.source,
            protocol: query.traffic.protocol,
            source_port: query.traffic.destination_port,
            destination_port: query.traffic.source_port,
        };
        for (i, id) in nodes.iter().enumerate() {
            let cfg = &old.nodes[id].configuration;
            let netmap = &new.nodes[id];
            let leased = Leased {
                netmap: &netmap.netmap,
                lease: &leases[id],
            };
            let previous = i.checked_sub(1).map(|p| nodes[p]);
            let next = nodes.get(i + 1).copied();
            let relay = i > 0 && i + 1 < nodes.len();
            let grant = &netmap.netmap.grants[key];
            assert_eq!(grant.services.is_none(), relay, "relay service visibility");
            assert!(grant.paths.iter().any(|p| &p.nodes == nodes));
            if relay {
                assert!(grant.paths.iter().all(|p| p.nodes.contains(id)));
            }
            let (old_forward, new_forward) = if relay {
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
                    local::admit_handshake(leased, key, nodes, query.source, previous, false, 1001),
                )
            } else {
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
                        key,
                        nodes,
                        query.source,
                        previous,
                        &query.traffic,
                        1001,
                    ),
                )
            };
            assert!(
                same_hop(old_forward, new_forward),
                "forward at {i} of {nodes:?}: {old_forward:?} vs {new_forward:?}"
            );
            assert!(new_forward.is_ok(), "{new_forward:?}");
            let (old_reply, new_reply) = if relay {
                (
                    old_admission::admit_handshake(
                        cfg,
                        query.id,
                        nodes,
                        query.source,
                        next,
                        true,
                        1001,
                    ),
                    local::admit_handshake(leased, key, nodes, query.source, next, true, 1001),
                )
            } else {
                (
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
                        key,
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
                same_hop(old_reply, new_reply),
                "reply at {i} of {nodes:?}: {old_reply:?} vs {new_reply:?}"
            );
            // Expired lease and a wrong previous hop are rejected locally.
            let expired = NetmapLease::new(leases[id].netmap, 0, 1001).unwrap();
            assert!(
                local::admit_handshake(
                    Leased {
                        netmap: &netmap.netmap,
                        lease: &expired
                    },
                    key,
                    nodes,
                    query.source,
                    previous,
                    false,
                    1001
                )
                .is_err()
            );
            assert!(
                local::admit_handshake(leased, key, nodes, query.source, Some(*id), false, 1001)
                    .is_err()
            );
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
                let traffic = Traffic {
                    source: if destination.is_ipv4() {
                        IpAddr::V4(source_address)
                    } else {
                        "fd00::1".parse().unwrap()
                    },
                    destination: *destination,
                    protocol: *protocol,
                    source_port: port_number.and(port(40_000)),
                    destination_port,
                };
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
                    traffic,
                });
            }
        }
    }
    let ids: Vec<RequestId> = demands.keys().copied().collect();
    let mut decisions = BTreeMap::new();
    let mut olds = Vec::new();
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
    let mut classified_keys: BTreeMap<RequestId, Vec<GrantKey>> = BTreeMap::new();
    for query in &queries {
        coverage.queries += 1;
        let (batch, decision) = &decisions[&query.id];
        let source = &new.nodes[&query.source].netmap;
        let classified = local::classify(
            source,
            query.traffic.destination,
            query.traffic.protocol,
            query.traffic.destination_port,
        );
        if let Ok(Destination::Mesh(key)) = &classified {
            classified_keys.insert(query.id, vec![*key]);
        } else if let Ok(Destination::Exit(keys)) = &classified {
            classified_keys.insert(query.id, keys.clone());
        }
        let context = || {
            format!(
                "seed {seed} {} -> {:?}: {decision:?} vs {classified:?}",
                query.source, query.traffic
            )
        };
        match (decision, &classified) {
            (DemandDecision::Permitted { permit }, Ok(Destination::Mesh(key))) => {
                coverage.mesh_permitted += 1;
                let grant = &source.grants[key];
                assert_eq!(
                    shape(
                        permit
                            .paths
                            .iter()
                            .map(|p| (p.nodes.clone(), p.dial_candidates.clone()))
                    ),
                    shape(
                        grant
                            .paths
                            .iter()
                            .map(|p| (p.nodes.clone(), p.dial_candidates.clone()))
                    ),
                    "{}",
                    context()
                );
                let GrantTarget::Mesh { node } = key.target else {
                    unreachable!()
                };
                let reverse = GrantKey {
                    source: node,
                    target: GrantTarget::Mesh { node: query.source },
                };
                if s.cfg.access.iter().any(|r| r.bidirectional)
                    && new.nodes[&node]
                        .netmap
                        .grants
                        .get(&reverse)
                        .is_some_and(|g| g.services.is_some())
                {
                    coverage.bidirectional_reverse += 1;
                }
                compare_hops(
                    &olds[*batch],
                    &new,
                    &leases,
                    query,
                    std::slice::from_ref(key),
                    permit,
                    coverage,
                );
            }
            (DemandDecision::Permitted { permit }, Ok(Destination::Exit(keys))) => {
                coverage.exit_permitted += 1;
                if keys.len() > 1 {
                    coverage.exit_backup += 1;
                }
                let paths: Vec<_> = keys
                    .iter()
                    .flat_map(|key| &source.grants[key].paths)
                    .map(|p| (p.nodes.clone(), p.dial_candidates.clone()))
                    .collect();
                assert_eq!(
                    shape(
                        permit
                            .paths
                            .iter()
                            .map(|p| (p.nodes.clone(), p.dial_candidates.clone()))
                    ),
                    paths,
                    "{}",
                    context()
                );
                let selection = &source.grants[&keys[0]].exit.as_ref().unwrap().selection;
                assert!(
                    keys.iter()
                        .all(|k| &source.grants[k].exit.as_ref().unwrap().selection == selection)
                );
                compare_hops(&olds[*batch], &new, &leases, query, keys, permit, coverage);
            }
            (DemandDecision::Direct, Ok(Destination::Direct)) => coverage.direct += 1,
            (DemandDecision::Unmanaged, Ok(Destination::Unmanaged)) => coverage.unmanaged += 1,
            (DemandDecision::Unavailable { .. }, Err(_)) => coverage.denied += 1,
            _ => panic!("decision mismatch: {}", context()),
        }
    }
    // The terminal admits exactly the flows the source classifies to the grant,
    // independent of which other destination brought the grant into being.
    for query in &queries {
        let source = &new.nodes[&query.source].netmap;
        for (key, grant) in source
            .grants
            .iter()
            .filter(|(k, _)| k.source == query.source)
        {
            let path = &grant.paths[0].nodes;
            let terminal = *path.last().unwrap();
            let leased = Leased {
                netmap: &new.nodes[&terminal].netmap,
                lease: &leases[&terminal],
            };
            let admitted = local::admit_forward(
                leased,
                key,
                path,
                query.source,
                Some(path[path.len() - 2]),
                &query.traffic,
                1001,
            )
            .is_ok();
            // Several Access scopes of one exit may cover the same flow; the
            // source picks the first, and the terminal accepts any of them.
            let exit_node = |k: &GrantKey| match k.target {
                GrantTarget::Exit { node, .. } => Some(node),
                GrantTarget::Mesh { .. } => None,
            };
            let expected = classified_keys.get(&query.id).is_some_and(|keys| {
                keys.contains(key)
                    || (exit_node(key).is_some()
                        && keys.iter().any(|k| exit_node(k) == exit_node(key))
                        && scope_covers(grant, &query.traffic))
            });
            assert_eq!(
                admitted, expected,
                "seed {seed} terminal {key:?} {:?}",
                query.traffic
            );
        }
    }
    // Relay copies carry no services; endpoint copies carry the full grant.
    for (id, compiled) in &new.nodes {
        for grant in compiled.netmap.grants.values() {
            let terminal = match grant.target {
                GrantTarget::Mesh { node } | GrantTarget::Exit { node, .. } => node,
            };
            assert_eq!(
                grant.services.is_some(),
                *id == grant.source || *id == terminal
            );
            let endpoint = &new.nodes[&grant.source].netmap.grants[&grant.key()];
            let own: BTreeSet<_> = grant.paths.iter().map(|p| &p.nodes).collect();
            assert!(
                own.iter()
                    .all(|p| endpoint.paths.iter().any(|e| &&e.nodes == p))
            );
        }
    }
}

fn scope_covers(grant: &Grant, traffic: &Traffic) -> bool {
    let exit = grant.exit.as_ref().unwrap();
    let inside = |prefix: &str| {
        prefix
            .parse::<ipnet::IpNet>()
            .is_ok_and(|net| net.contains(&traffic.destination))
    };
    exit.routes.iter().any(|route| inside(route))
        && match &exit.destination {
            ExitDestinationScope::Internet => {
                meshway_policy::is_internet_address(traffic.destination)
            }
            ExitDestinationScope::Cidr { prefix } => inside(prefix),
        }
        && grant
            .services
            .as_ref()
            .unwrap()
            .contains(traffic.protocol, traffic.destination_port)
}

#[test]
fn netmap_admission_equals_per_demand_compilation_for_generated_configurations() {
    let mut coverage = Coverage::default();
    for seed in 0..64 {
        check(seed, &mut coverage);
    }
    eprintln!("{coverage:#?}");
    assert_eq!(coverage.scenarios, 64);
    for (name, value) in [
        ("mesh", coverage.mesh_permitted),
        ("relayed", coverage.relayed),
        ("exit", coverage.exit_permitted),
        ("exit backup", coverage.exit_backup),
        ("direct", coverage.direct),
        ("unmanaged", coverage.unmanaged),
        ("denied", coverage.denied),
        ("bidirectional", coverage.bidirectional_reverse),
        ("disabled", coverage.disabled),
        ("unregistered", coverage.unregistered),
    ] {
        assert!(value > 0, "generated configurations never covered {name}");
    }
}
