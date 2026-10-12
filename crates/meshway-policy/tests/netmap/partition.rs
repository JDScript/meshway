//! The Node-side interval partition against two independent references: the
//! prefix subtraction that Server netmaps carried before R3, and first-match
//! of every address (`egress_rule_for_ip`) — over inline CIDRs and artifacts.
use super::*;
use ipnet::{IpNet, Ipv4Net};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

/// Deterministic xorshift; no test depends on platform randomness.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Random IPv4 prefixes clustered in 10.0.0.0/8 so that rules overlap.
fn prefixes(rng: &mut Rng, count: usize) -> Vec<String> {
    (0..count)
        .map(|_| {
            let len = 8 + rng.below(17) as u8;
            let address = Ipv4Addr::from(0x0a00_0000 | (rng.next() as u32 & 0x00ff_ffff));
            Ipv4Net::new(address, len).unwrap().trunc().to_string()
        })
        .collect()
}

fn artifact(cidrs: &[String]) -> Arc<meshway_rules::RuleSet> {
    Arc::new(
        meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
            ip_cidr: cidrs,
            ..Default::default()
        })
        .unwrap(),
    )
}

/// The pre-R3 Server algorithm: subtract each rule's prefixes, in order,
/// from the remaining prefixes by halving. Reference only.
fn subtract(net: IpNet, cut: IpNet, rest: &mut Vec<IpNet>, hit: &mut Vec<IpNet>) {
    if cut.contains(&net) {
        hit.push(net);
    } else if net.contains(&cut) {
        for child in net.subnets(net.prefix_len() + 1).unwrap() {
            subtract(child, cut, rest, hit);
        }
    } else {
        rest.push(net);
    }
}

fn prefix_partition(rules: &[(Vec<IpNet>, bool)], fallback_exit: bool) -> Vec<(IpNet, bool)> {
    let mut remaining: Vec<IpNet> = vec!["0.0.0.0/0".parse().unwrap()];
    let mut routes = Vec::new();
    for (nets, exit) in rules {
        for cut in nets {
            let (mut rest, mut hit) = (Vec::new(), Vec::new());
            for net in remaining {
                subtract(net, *cut, &mut rest, &mut hit);
            }
            routes.extend(hit.into_iter().map(|net| (net, *exit)));
            remaining = rest;
        }
    }
    routes.extend(remaining.into_iter().map(|net| (net, fallback_exit)));
    routes
}

fn scenario(rules: Vec<EgressRule>, fallback: RouteAction) -> Scenario {
    let mut s = base(4);
    s.cfg.access = vec![access(
        &[sel(1)],
        &["autogroup:internet".into()],
        &["*"],
        false,
    )];
    s.cfg
        .routing
        .egress
        .profiles
        .insert("p".into(), RoutingProfile { rules, fallback });
    s.cfg.routing.egress.assignments.push(ProfileAssignment {
        target: vec![sel(1)],
        allowed: vec!["p".into()],
        default: "p".into(),
        locked: false,
    });
    s
}

fn exit(action: &EgressAction) -> bool {
    matches!(action, EgressAction::Exit { .. })
}

#[test]
fn interval_partition_equals_prefix_subtraction_and_first_match() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    for case in 0..40 {
        let mut sets = RuleSets::new();
        let mut artifacts = Artifacts::new();
        let mut rules = Vec::new();
        let mut reference = Vec::new();
        for index in 0..1 + rng.below(6) as usize {
            let count = rng.below(4) as usize;
            let own = prefixes(&mut rng, count);
            let count = 1 + rng.below(60) as usize;
            let referenced = prefixes(&mut rng, count);
            let key = format!("s{index}");
            let set = artifact(&referenced);
            artifacts.insert(set.digest(), set.clone());
            sets.insert(key.clone(), set);
            let to_exit = rng.below(2) == 0;
            rules.push(EgressRule {
                remark: String::new(),
                matcher: RouteMatch {
                    ip_cidr: own.clone(),
                    rule_sets: vec![key],
                    ..Default::default()
                },
                action: if to_exit {
                    exit_action(&[4], &[3])
                } else {
                    RuleAction::Direct
                },
            });
            let nets = own
                .iter()
                .chain(&referenced)
                .map(|net| net.parse().unwrap())
                .collect();
            reference.push((nets, to_exit));
        }
        let fallback_exit = case % 2 == 0;
        let fallback = if fallback_exit {
            RouteAction::Exit {
                nodes: vec![sel(4)],
                via: vec![sel(3)],
            }
        } else {
            RouteAction::Unmanaged
        };
        let s = scenario(rules.clone(), fallback);
        let compiled = compile_netmap(NetmapInput {
            network: network(),
            revision: Revision::MIN,
            configuration: &s.cfg,
            subjects: &s.subjects,
            inventory: &s.inventory,
            rule_sets: &sets,
            selected_profiles: &BTreeMap::new(),
            selection_versions: &s.selection_versions,
            previous: &BTreeMap::new(),
        })
        .unwrap();
        let netmap = &compiled.nodes[&node(1)].netmap;
        let compiler = LocalCompiler::new(netmap, &artifacts);
        let partition = compiler.partition(node(1)).unwrap().unwrap();
        // Server and Node agree on the selection, availability included.
        assert_eq!(
            Some(&compiler.egress(node(1)).unwrap()),
            netmap.egress.as_ref()
        );
        let ranges: Vec<_> = partition.ipv4().collect();

        // Every old disjoint prefix lies in one interval with its action.
        for (net, to_exit) in prefix_partition(&reference, fallback_exit) {
            let IpNet::V4(net) = net else { unreachable!() };
            let holder = ranges
                .iter()
                .find(|(range, _)| range.contains(&net.network()))
                .unwrap();
            assert!(
                holder.0.contains(&net.broadcast()),
                "case {case}: {net} split"
            );
            assert_eq!(exit(holder.1), to_exit, "case {case}: {net}");
        }

        // Range boundaries and random addresses decide like first-match.
        let mut probes: Vec<Ipv4Addr> = ranges
            .iter()
            .flat_map(|(range, _)| [*range.start(), *range.end()])
            .collect();
        probes.extend(
            (0..200).map(|_| Ipv4Addr::from(0x0a00_0000 | (rng.next() as u32 & 0x00ff_ffff))),
        );
        for address in probes {
            let hit = compiler
                .egress_rule_for_ip(node(1), IpAddr::V4(address))
                .unwrap();
            let expected = match hit {
                Some(hit) => matches!(hit.action, RouteAction::Exit { .. }),
                None => fallback_exit,
            };
            assert_eq!(
                exit(partition.action(IpAddr::V4(address))),
                expected,
                "case {case}: {address}"
            );
        }

        // The capture boundary is the merged exit ranges, as the summary counts.
        let capture = compiler.capture().unwrap().unwrap();
        let summary = partition.summary();
        assert_eq!(capture.managed_ipv4.len(), summary.managed_ipv4 as usize);
        let managed: u64 = ranges
            .iter()
            .filter(|(_, action)| exit(action))
            .map(|(range, _)| u64::from(u32::from(*range.end()) - u32::from(*range.start())) + 1)
            .sum();
        let captured: u64 = capture
            .managed_ipv4
            .iter()
            .map(|(start, end)| u64::from(u32::from(*end) - u32::from(*start)) + 1)
            .sum();
        assert_eq!(managed, captured);
        for pair in capture.managed_ipv4.windows(2) {
            assert!(
                u32::from(pair[0].1) + 1 < u32::from(pair[1].0),
                "adjacent ranges merge"
            );
        }
    }
}

#[test]
fn a_missing_artifact_fails_closed_and_a_later_rule_is_never_reached() {
    let set = artifact(&["10.1.0.0/16".into()]);
    let sets: RuleSets = [("s0".to_owned(), set)].into();
    let rules = vec![
        EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                rule_sets: vec!["s0".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        },
        egress_rule("10.0.0.0/8", RuleAction::Direct),
    ];
    let s = scenario(rules, RouteAction::Unmanaged);
    let compiled = compile_netmap(NetmapInput {
        network: network(),
        revision: Revision::MIN,
        configuration: &s.cfg,
        subjects: &s.subjects,
        inventory: &s.inventory,
        rule_sets: &sets,
        selected_profiles: &BTreeMap::new(),
        selection_versions: &s.selection_versions,
        previous: &BTreeMap::new(),
    })
    .unwrap();
    let compiler = LocalCompiler::new(&compiled.nodes[&node(1)].netmap, &Artifacts::new());
    assert_eq!(
        compiler.partition(node(1)).unwrap(),
        Err(UnavailableReason::MissingDependency)
    );
    let capture = compiler.capture().unwrap().unwrap();
    assert_eq!(
        capture.unavailable,
        Some(UnavailableReason::MissingDependency)
    );
    assert!(capture.default_capture);
    assert_eq!(
        capture.managed_ipv4,
        [(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)]
    );
    assert_eq!(
        compiler.decide(
            node(1),
            "10.2.0.1".parse().unwrap(),
            BusinessProtocol::Tcp,
            port(443)
        ),
        Ok(LocalDecision::Denied(UnavailableReason::MissingDependency))
    );
}
