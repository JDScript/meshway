//! IPv6 exit (NAT66): an exit carries IPv6 only by configuration
//! (`exitFamilies`) and its executor; the source captures, decides and
//! admits each family on its own; IPv4 behaviour is unchanged.
use super::*;
use meshway_model::exit::ExitFamily;
use meshway_policy::admission::{AdmissionError, Hop, Traffic, netmap::admit_forward};
use std::net::{IpAddr, Ipv6Addr};

const PUBLIC6: &str = "3ffe:1:2:3::7";

/// Dual-stack source 1 may reach the Internet on tcp:443 and exits
/// everything through `[4, 5]` (4 first).
fn setup() -> Scenario {
    let mut s = base(5);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into()],
        &["tcp:443"],
        false,
    )];
    s.cfg.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![sel(4), sel(5)],
                via: vec![],
            },
        },
    );
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    });
    dual_stack(&mut s, &[Some(1), Some(2), Some(3), Some(4), Some(5)]);
    s
}

fn compiler(compiled: &CompiledNetmaps, at: u8) -> LocalCompiler {
    LocalCompiler::new(&compiled.nodes[&node(at)].netmap, &Default::default())
}

fn decide(compiled: &CompiledNetmaps, destination: &str) -> LocalDecision {
    compiler(compiled, 1)
        .decide(
            node(1),
            destination.parse().unwrap(),
            BusinessProtocol::Tcp,
            port(443),
        )
        .unwrap()
}

fn exits_of(decision: &LocalDecision) -> Vec<NodeId> {
    let LocalDecision::Permitted(decision) = decision else {
        panic!("{decision:?}")
    };
    let mut exits: Vec<_> = decision
        .paths
        .iter()
        .map(|path| *path.nodes.last().unwrap())
        .collect();
    exits.dedup();
    exits
}

fn contains(ranges: &[(Ipv6Addr, Ipv6Addr)], address: &str) -> bool {
    let address: Ipv6Addr = address.parse().unwrap();
    ranges
        .iter()
        .any(|(start, end)| *start <= address && address <= *end)
}

#[test]
fn ipv6_stays_sunk_until_an_exit_carries_it_by_configuration_and_executor() {
    let s = setup();
    let compiled = netmaps(&s);
    // Default `exitFamilies`: an IPv4 role, IPv6 sunk and statically refused.
    assert_eq!(
        compiled.nodes[&node(4)]
            .netmap
            .exit_role
            .as_ref()
            .unwrap()
            .families,
        [ExitFamily::Ipv4]
    );
    let capture = compiler(&compiled, 1).capture().unwrap().unwrap();
    assert!(capture.managed_ipv6.is_empty());
    assert!(contains(&capture.sink_ipv6, PUBLIC6));
    assert_eq!(
        decide(&compiled, PUBLIC6),
        LocalDecision::Denied(UnavailableReason::UnsupportedFamily)
    );
    // IPv4 is unchanged by families.
    assert_eq!(exits_of(&decide(&compiled, "8.8.8.8")), [node(4), node(5)]);

    // Configured but without the executor: captured, and no exit can carry it.
    let mut configured = s.clone();
    ipv6_exit(&mut configured, 5, false);
    let compiled = netmaps(&configured);
    assert_eq!(
        compiled.nodes[&node(5)]
            .netmap
            .exit_role
            .as_ref()
            .unwrap()
            .families,
        [ExitFamily::Ipv4]
    );
    let capture = compiler(&compiled, 1).capture().unwrap().unwrap();
    assert!(contains(&capture.managed_ipv6, PUBLIC6));
    assert!(!contains(&capture.sink_ipv6, PUBLIC6));
    assert_eq!(
        decide(&compiled, PUBLIC6),
        LocalDecision::Denied(UnavailableReason::ExitUnavailable)
    );

    // With the executor: an IPv4+IPv6 role, a new role identity.
    let mut capable = s.clone();
    ipv6_exit(&mut capable, 5, true);
    let compiled = netmaps(&capable);
    let role = compiled.nodes[&node(5)].netmap.exit_role.clone().unwrap();
    assert_eq!(role.families, [ExitFamily::Ipv4, ExitFamily::Ipv6]);
    assert_ne!(
        Some(role.generation),
        netmaps(&s).nodes[&node(5)]
            .netmap
            .exit_role
            .as_ref()
            .map(|role| role.generation)
    );
    let projection = &compiled.nodes[&node(1)].netmap.projection;
    assert!(projection.eligible_exits.iter().any(|exit| {
        exit.node == node(5) && exit.families == [ExitFamily::Ipv4, ExitFamily::Ipv6]
    }));
    // The two families take different exits of the same set: IPv6 only 5.
    assert_eq!(exits_of(&decide(&compiled, PUBLIC6)), [node(5)]);
    assert_eq!(exits_of(&decide(&compiled, "8.8.8.8")), [node(4), node(5)]);
}

#[test]
fn a_source_without_mesh_ipv6_sinks_ipv6_and_forbidden_destinations_never_exit() {
    let mut s = setup();
    ipv6_exit(&mut s, 4, true);
    let mut without = s.clone();
    without.cfg.nodes.get_mut(&node(1)).unwrap().addresses.ipv6 = None;
    let compiled = netmaps(&without);
    let capture = compiler(&compiled, 1).capture().unwrap().unwrap();
    assert!(capture.managed_ipv6.is_empty());
    assert!(contains(&capture.sink_ipv6, PUBLIC6));
    assert_eq!(
        decide(&compiled, PUBLIC6),
        LocalDecision::Denied(UnavailableReason::UnsupportedFamily)
    );
    let compiled = netmaps(&s);
    assert_eq!(exits_of(&decide(&compiled, PUBLIC6)), [node(4)]);
    // Mapped IPv4, NAT64, link-local, multicast, loopback: never through an exit.
    for forbidden in [
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
        "fe80::1",
        "ff0e::1",
        "::1",
    ] {
        assert!(
            matches!(decide(&compiled, forbidden), LocalDecision::Denied(_)),
            "{forbidden}"
        );
    }
    // A Mesh pool address is never external.
    assert_eq!(
        decide(&compiled, "fd79::99"),
        LocalDecision::Denied(UnavailableReason::NoPath)
    );
}

#[test]
fn a_source_naming_itself_resolves_each_family_against_the_exits_that_carry_it() {
    let mut s = setup();
    s.cfg
        .routing
        .egress
        .profiles
        .get_mut("exit")
        .unwrap()
        .fallback = RouteAction::Exit {
        nodes: vec![sel(4), sel(1)],
        via: vec![],
    };
    // 4 carries only IPv4: IPv6 of source 1 is its own direct egress.
    let compiled = netmaps(&s);
    assert_eq!(decide(&compiled, PUBLIC6), LocalDecision::Direct);
    assert_eq!(exits_of(&decide(&compiled, "8.8.8.8")), [node(4)]);
    let capture = compiler(&compiled, 1).capture().unwrap().unwrap();
    assert!(!contains(&capture.managed_ipv6, PUBLIC6) && !contains(&capture.sink_ipv6, PUBLIC6));
    ipv6_exit(&mut s, 4, true);
    let compiled = netmaps(&s);
    assert_eq!(exits_of(&decide(&compiled, PUBLIC6)), [node(4)]);
}

#[test]
fn the_exit_admits_only_the_families_its_role_carries() {
    let mut s = setup();
    ipv6_exit(&mut s, 4, true);
    let compiled = netmaps(&s);
    let LocalDecision::Permitted(decision) = decide(&compiled, PUBLIC6) else {
        panic!()
    };
    let path = &decision.paths[0].nodes;
    let traffic = Traffic {
        source: "fd79::1".parse().unwrap(),
        destination: PUBLIC6.parse().unwrap(),
        protocol: BusinessProtocol::Tcp,
        source_port: port(1234),
        destination_port: port(443),
    };
    let at_exit = &compiled.nodes[path.last().unwrap()].netmap;
    let previous = path.len().checked_sub(2).map(|i| path[i]);
    let decision = match compiler(&compiled, 4)
        .decide(
            node(1),
            traffic.destination,
            BusinessProtocol::Tcp,
            port(443),
        )
        .unwrap()
    {
        LocalDecision::Permitted(decision) => decision,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        admit_forward(at_exit, &decision, path, node(1), previous, &traffic),
        Ok(Hop::Exit)
    );
    // A forged mixed-family packet is not its traffic.
    let forged = Traffic {
        source: "100.64.0.1".parse().unwrap(),
        ..traffic.clone()
    };
    assert_eq!(
        admit_forward(at_exit, &decision, path, node(1), previous, &forged),
        Err(AdmissionError::Traffic)
    );
    // The same packet at an exit whose role is IPv4 only.
    let mut v4 = (**at_exit).clone();
    v4.exit_role.as_mut().unwrap().families = vec![ExitFamily::Ipv4];
    assert_eq!(
        admit_forward(&v4, &decision, path, node(1), previous, &traffic),
        Err(AdmissionError::Traffic)
    );
    let _: IpAddr = traffic.destination;
}
