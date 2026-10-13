//! Relay candidates: every two-hop relay the topology allows is a candidate,
//! whatever its Node ID, and the search stays within its shared budget.
use super::*;
use std::net::IpAddr;

fn meshed(count: u8) -> Scenario {
    let mut s = base(count);
    s.cfg.access = vec![access(&["*".into()], &["*".into()], &["*"], false)];
    s
}

fn paths(compiled: &CompiledNetmaps, source: u8, target: u8) -> Vec<Vec<NodeId>> {
    let local = LocalCompiler::new(&compiled.nodes[&node(source)].netmap, &Default::default());
    let address: IpAddr = format!("100.64.0.{target}").parse().unwrap();
    match local
        .decide(node(source), address, BusinessProtocol::IcmpEcho, None)
        .unwrap()
    {
        LocalDecision::Permitted(decision) => {
            decision.paths.into_iter().map(|path| path.nodes).collect()
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_two_hop_relay_is_a_candidate_beyond_the_first_ids() {
    let compiled = netmaps(&meshed(12));
    let found = paths(&compiled, 1, 12);
    let mut expected = vec![vec![node(1), node(12)]];
    expected.extend((2..=11).map(|relay| vec![node(1), node(relay), node(12)]));
    // Ten relays exceed the eight candidates a source keeps; Policy still
    // lists every one of that length (the source ranks them), and nothing longer.
    assert_eq!(found, expected);
    // A relay far down the ID order admits the path it is offered.
    let relay = LocalCompiler::new(&compiled.nodes[&node(11)].netmap, &Default::default());
    let target = Target::Mesh { node: node(12) };
    assert!(
        relay
            .transit(node(1), &target)
            .unwrap()
            .iter()
            .any(|path| path.nodes == [node(1), node(11), node(12)])
    );
}

#[test]
fn fewer_than_enough_short_paths_continue_to_the_next_length() {
    let mut s = meshed(6);
    // 1 reaches 6 only through relays; 1 reaches 2 only directly.
    s.cfg.defaults.topology = TopologyDefault::Deny;
    for (a, b) in [(1, 2), (1, 3), (2, 6), (3, 4), (4, 6), (3, 6)] {
        s.cfg.topology.push(TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: sel(a),
            b: sel(b),
            transports: Input::Omitted,
            keepalive: false,
        });
    }
    let found = paths(&netmaps(&s), 1, 6);
    assert_eq!(
        found,
        vec![
            vec![node(1), node(2), node(6)],
            vec![node(1), node(3), node(6)],
            vec![node(1), node(3), node(4), node(6)],
        ]
    );
}

#[test]
fn a_large_projection_lists_every_relay_within_the_budget() {
    let count = 250;
    let compiled = netmaps(&meshed(count));
    let found = paths(&compiled, 1, count);
    assert_eq!(found.len(), usize::from(count) - 1);
    assert_eq!(found[0], [node(1), node(count)]);
    assert_eq!(
        found.last().unwrap().as_slice(),
        [node(1), node(count - 1), node(count)]
    );
    assert!(found[1..].iter().all(|path| path.len() == 3));
}
