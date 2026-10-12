//! Independent projection oracle: the same compiler receives all stable input
//! in one case and the production visibility projection in the other. There is
//! no retired demand compiler or second policy implementation in this test.
use super::*;
use std::{net::IpAddr, sync::Arc};

#[test]
fn projected_and_full_configuration_have_identical_local_admission_and_paths() {
    let mut queries = 0;
    let mut permitted = 0;
    let mut denied = 0;
    for seed in 0..64 {
        let scenario = generated(seed);
        let compiled = netmaps(&scenario);
        let destinations: Vec<IpAddr> = scenario
            .cfg
            .nodes
            .values()
            .flat_map(|node| {
                node.addresses
                    .ipv4
                    .map(IpAddr::V4)
                    .into_iter()
                    .chain(node.addresses.ipv6.map(IpAddr::V6))
            })
            .chain(
                [
                    "1.1.1.1",
                    "8.8.8.8",
                    "203.0.113.7",
                    "2001:4860:4860::8888",
                    "100.64.0.254",
                ]
                .map(|ip| ip.parse().unwrap()),
            )
            .collect();
        for (source, projected) in &compiled.nodes {
            let mut full = projected.netmap.as_ref().clone();
            full.projection.configuration = ConfigurationView::from(&scenario.cfg);
            full.projection.subjects = scenario.subjects.clone();
            full.projection.inventory = scenario.inventory.clone();
            full.projection.selection_versions = scenario.selection_versions.clone();
            let full = LocalCompiler::from_shared(Arc::new(full), &Default::default());
            let local = LocalCompiler::from_shared(projected.netmap.clone(), &Default::default());
            for destination in &destinations {
                for (protocol, port) in [
                    (BusinessProtocol::IcmpEcho, None),
                    (BusinessProtocol::Tcp, port(22)),
                    (BusinessProtocol::Tcp, port(80)),
                    (BusinessProtocol::Tcp, port(443)),
                    (BusinessProtocol::Tcp, port(42425)),
                    (BusinessProtocol::Tcp, port(65535)),
                    (BusinessProtocol::Udp, port(53)),
                    (BusinessProtocol::Udp, port(443)),
                    (BusinessProtocol::Udp, port(42424)),
                ] {
                    let actual = local.decide(*source, *destination, protocol, port).unwrap();
                    let expected = full.decide(*source, *destination, protocol, port).unwrap();
                    queries += 1;
                    match (&actual, &expected) {
                        (LocalDecision::Denied(_), LocalDecision::Denied(_)) => denied += 1,
                        _ => assert_eq!(
                            actual, expected,
                            "seed={seed} source={source} dst={destination} service={protocol:?}:{port:?}"
                        ),
                    }
                    if let LocalDecision::Permitted(decision) = actual {
                        permitted += 1;
                        for path in &decision.paths {
                            for relay in path
                                .nodes
                                .iter()
                                .skip(1)
                                .take(path.nodes.len().saturating_sub(2))
                            {
                                let relay = LocalCompiler::from_shared(
                                    compiled.nodes[relay].netmap.clone(),
                                    &Default::default(),
                                );
                                assert_eq!(
                                    relay.transit(*source, &decision.target).unwrap(),
                                    full.transit(*source, &decision.target).unwrap()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        queries > 27_000 && permitted > 0 && denied > 0,
        "{queries} queries; {permitted} permitted; {denied} denied"
    );
}
